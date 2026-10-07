#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

pub mod fakes;

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
    checked(output)
}

/// Like `bilbo`, with `input` on stdin.
pub fn bilbo_input(cwd: &Path, env: &[(&str, &str)], args: &[&str], input: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bilbo"))
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(cwd)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin);
    checked(child.wait_with_output().unwrap())
}

/// Runs bilbo with stdout and stderr on two pseudo-terminals `cols` columns wide and stdin empty, as a person's
/// terminal; `\r\n` reads back as `\n`. Unlike `bilbo`, it does not require the `bilbo: ` prefix: a terminal may
/// get level marks.
pub fn bilbo_tty(cwd: &Path, env: &[(&str, &str)], args: &[&str], cols: u16) -> Run {
    let (out_master, out_slave) = pty(cols);
    let (err_master, err_slave) = pty(cols);
    let mut child = Command::new(env!("CARGO_BIN_EXE_bilbo"))
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_slave))
        .stderr(Stdio::from(err_slave))
        .spawn()
        .unwrap();
    let read = |master: OwnedFd| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = std::fs::File::from(master).read_to_end(&mut bytes);
            String::from_utf8(bytes).unwrap().replace("\r\n", "\n")
        })
    };
    let (out, err) = (read(out_master), read(err_master));
    let code = child.wait().unwrap().code().unwrap();
    Run {
        code,
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
    }
}

/// A pseudo-terminal `cols` columns wide, as its master and slave. Both are close-on-exec, so a child another
/// test thread spawns never holds a slave open.
fn pty(cols: u16) -> (OwnedFd, OwnedFd) {
    let (mut master, mut slave) = (0, 0);
    let mut size = libc::winsize {
        ws_row: 50,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    };
    assert_eq!(opened, 0, "openpty failed");
    for fd in [master, slave] {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

fn checked(output: std::process::Output) -> Run {
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
    delay: Duration,
    loading: usize,
    health_checks: usize,
    answered: usize,
    requests: Vec<Request>,
}

/// An embedder on 127.0.0.1 answering `POST /v1/embeddings` and `GET /health`, one request per
/// connection.
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
        Fake::spawn(dims, addr, Socket::Listening(listener), None)
    }

    /// Like `start`, but listens only once `trigger` exists, the way a server the service manager
    /// starts comes up. The port is bound from the start and never released, so no other test can
    /// take it; connecting fails until the trigger appears. Stops at drop even if never
    /// triggered.
    pub fn start_when(dims: usize, trigger: &Path) -> Fake {
        let (fd, addr) = reserve();
        Fake::spawn(
            dims,
            addr,
            Socket::Reserved(fd),
            Some(trigger.to_path_buf()),
        )
    }

    fn spawn(dims: usize, addr: SocketAddr, socket: Socket, trigger: Option<PathBuf>) -> Fake {
        let state = Arc::new(Mutex::new(State {
            dims,
            table: Vec::new(),
            fail_after: None,
            status: None,
            too_few: false,
            stall: false,
            delay: Duration::ZERO,
            loading: 0,
            health_checks: 0,
            answered: 0,
            requests: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = Arc::clone(&state);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                if let Some(trigger) = trigger {
                    while !trigger.exists() {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                }
                let listener = socket.listen();
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

    /// The port of `url`.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Answer the next `n` `GET /health` requests with 503 (llama-server loading).
    pub fn loading(&self, n: usize) {
        self.state.lock().unwrap().loading = n;
    }

    /// How many `GET /health` requests came.
    pub fn health_checks(&self) -> usize {
        self.state.lock().unwrap().health_checks
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

    /// Wait `delay` before answering each embeddings request.
    pub fn delay(&self, delay: Duration) {
        self.state.lock().unwrap().delay = delay;
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
        state.delay = Duration::ZERO;
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
        if let Some(thread) = self.thread.take() {
            // The thread may not listen yet: poke until it has seen `stop` and left.
            while !thread.is_finished() {
                let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(50));
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = thread.join();
        }
    }
}

enum Socket {
    Listening(TcpListener),
    /// Bound but not yet `listen`ing: connecting is refused, and the port stays ours.
    Reserved(OwnedFd),
}

impl Socket {
    fn listen(self) -> TcpListener {
        match self {
            Socket::Listening(listener) => listener,
            Socket::Reserved(fd) => {
                let rc = unsafe { libc::listen(fd.as_raw_fd(), 128) };
                assert_eq!(
                    rc,
                    0,
                    "fake embedder cannot listen: {}",
                    std::io::Error::last_os_error()
                );
                TcpListener::from(fd)
            }
        }
    }
}

/// A TCP socket bound to a free port on 127.0.0.1 that does not listen.
fn reserve() -> (OwnedFd, SocketAddr) {
    unsafe {
        let raw = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        assert!(raw >= 0, "{}", std::io::Error::last_os_error());
        let fd = OwnedFd::from_raw_fd(raw);
        let mut sin: libc::sockaddr_in = std::mem::zeroed();
        sin.sin_family = libc::AF_INET as libc::sa_family_t;
        sin.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
        let size = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        let rc = libc::bind(raw, &sin as *const _ as *const libc::sockaddr, size);
        assert_eq!(rc, 0, "{}", std::io::Error::last_os_error());
        let mut len = size;
        let rc = libc::getsockname(raw, &mut sin as *mut _ as *mut libc::sockaddr, &mut len);
        assert_eq!(rc, 0, "{}", std::io::Error::last_os_error());
        let addr = SocketAddr::from(([127, 0, 0, 1], u16::from_be(sin.sin_port)));
        (fd, addr)
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
    if method == "GET" && path == "/health" {
        let (code, text) = {
            let mut state = shared.lock().unwrap();
            state.health_checks += 1;
            if state.loading > 0 {
                state.loading -= 1;
                (
                    503,
                    r#"{"error":{"message":"Loading model","type":"unavailable_error","code":503}}"#,
                )
            } else {
                (200, r#"{"status":"ok"}"#)
            }
        };
        respond(stream, code, text);
        return;
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
    let delay = shared.lock().unwrap().delay;
    if !wrong && !delay.is_zero() {
        std::thread::sleep(delay);
    }
    respond(stream, code, &text);
}

fn respond(mut stream: TcpStream, code: u16, text: &str) {
    let reason = match code {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
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

/// One answer of the page server: a status, headers and body bytes. `Content-Length` and
/// `Connection: close` are added when it is sent.
#[derive(Clone, Debug)]
pub struct Route {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Route {
    /// A 200 answer with `content_type` and `body`.
    pub fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Route {
        Route::status(200)
            .header("Content-Type", content_type)
            .body(body)
    }

    /// An answer with `status` and no headers or body.
    pub fn status(status: u16) -> Route {
        Route {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// A 301 answer to `location`.
    pub fn redirect(location: &str) -> Route {
        Route::status(301).header("Location", location)
    }

    pub fn header(mut self, name: &str, value: &str) -> Route {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Route {
        self.body = body.into();
        self
    }
}

/// A page server on 127.0.0.1 answering `GET` from a table of paths, one request per connection.
/// A path not in the table gets a 404.
pub struct Pages {
    /// `http://127.0.0.1:<port>`, no trailing slash.
    pub url: String,
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Pages {
    /// Serves `routes`, each a path (with its query, as sent) and its answer, from a thread.
    pub fn start(routes: Vec<(&str, Route)>) -> Pages {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let routes: Vec<(String, Route)> = routes
            .into_iter()
            .map(|(path, route)| (path.to_string(), route))
            .collect();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let seen = Arc::clone(&seen);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        serve_page(stream, &routes, &seen);
                    }
                }
            })
        };
        Pages {
            url: format!("http://{addr}"),
            addr,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    /// The URL of `path`, which starts with `/`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    /// The paths asked for, in order.
    pub fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Pages {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // Poke the blocked accept so the thread sees `stop` and leaves.
            while !thread.is_finished() {
                let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(50));
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = thread.join();
        }
    }
}

fn serve_page(mut stream: TcpStream, routes: &[(String, Route)], seen: &Mutex<Vec<String>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
    }
    seen.lock().unwrap().push(path.clone());
    let missing = Route::status(404);
    let route = routes
        .iter()
        .find(|(p, _)| *p == path)
        .map_or(&missing, |(_, route)| route);
    let reason = match route.status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", route.status);
    for (name, value) in &route.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        route.body.len()
    ));
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&route.body);
    let _ = stream.flush();
}

static UNUSED: Mutex<Vec<OwnedFd>> = Mutex::new(Vec::new());

/// A port on 127.0.0.1 that nothing listens on and that no other test or process can take: the
/// socket is bound and kept for the rest of the process, but never listens. A connect to it fails
/// (refused on Linux, silently dropped on macOS, so it only times out there).
pub fn unused_port() -> u16 {
    let (fd, addr) = reserve();
    UNUSED.lock().unwrap().push(fd);
    addr.port()
}

/// A URL that refuses at once: bind 127.0.0.1:0, keep the port, drop the listener.
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

const BENCH_SECTIONS: usize = 6;

const BENCH_WORDS: &[&str] = &[
    "embedder",
    "timeout",
    "decisão",
    "ação",
    "configuração",
    "memória",
    "código",
    "índice",
    "sessão",
    "versão",
    "também",
    "não",
    "função",
    "próximo",
    "rollback",
    "cache",
    "store",
    "note",
    "index",
    "query",
    "passage",
    "heading",
    "ranking",
    "score",
    "token",
    "buffer",
    "thread",
    "queue",
    "retry",
    "backoff",
    "latency",
    "deploy",
    "release",
    "branch",
    "commit",
    "review",
    "agent",
    "prompt",
    "model",
    "context",
    "window",
    "limit",
    "batch",
    "worker",
    "stream",
    "socket",
    "client",
    "server",
    "request",
    "response",
    "schema",
    "migration",
    "column",
    "table",
    "record",
    "field",
    "value",
    "string",
    "number",
    "array",
    "object",
    "module",
    "package",
    "crate",
    "library",
    "compiler",
    "runtime",
    "memory",
    "storage",
    "decisão",
    "solução",
    "organização",
    "informação",
    "atenção",
    "relação",
    "operação",
    "documentação",
    "integração",
    "execução",
    "validação",
    "descrição",
    "condição",
    "posição",
    "variável",
    "método",
    "análise",
    "técnica",
    "prática",
    "histórico",
    "automático",
    "dinâmico",
    "estático",
    "através",
    "além",
    "então",
    "porém",
    "já",
    "até",
    "você",
    "são",
    "está",
    "será",
    "podem",
    "devem",
    "quando",
    "depois",
    "antes",
    "sempre",
    "nunca",
    "porque",
    "enquanto",
    "durante",
    "entre",
    "sobre",
    "sem",
    "com",
    "para",
    "the",
    "and",
    "with",
    "from",
    "into",
    "over",
    "under",
    "while",
    "after",
    "before",
    "because",
    "should",
    "would",
    "could",
    "might",
    "every",
    "other",
    "which",
    "their",
    "about",
    "first",
    "last",
    "next",
    "same",
    "each",
    "only",
];

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn bench_text(seed: &mut u64, words: usize) -> String {
    let picked: Vec<&str> = (0..words)
        .map(|_| BENCH_WORDS[(xorshift(seed) % BENCH_WORDS.len() as u64) as usize])
        .collect();
    picked.join(" ")
}

/// A paragraph of 40 to 120 words.
fn paragraph(seed: &mut u64) -> String {
    let words = 40 + (xorshift(seed) % 81) as usize;
    bench_text(seed, words)
}

/// A generated store of 450 notes, about 6 MiB, for the timing tests; returns the root and its size in MiB.
pub fn bench_store(dir: &TempDir) -> (PathBuf, f64) {
    const KINDS: [&str; 9] = [
        "plan",
        "spec",
        "design",
        "decision",
        "gotcha",
        "research",
        "review",
        "report",
        "reference",
    ];
    let root = store(dir);
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut total = 0usize;
    for i in 0..450 {
        let mut text = format!(
            "---\nid: {}\ncreated: 2026-10-02T14:23-03:00\n---\n\n# Bench note {i}\n",
            IDS[0]
        );
        for section in 0..BENCH_SECTIONS {
            text.push_str(&format!(
                "\n## Section {section}\n\n{}\n",
                paragraph(&mut seed)
            ));
            for sub in 0..2 + (section + i) % 3 {
                text.push_str(&format!("\n### Part {sub}\n\n{}\n", paragraph(&mut seed)));
            }
            if section % 3 == 0 {
                text.push_str("\n```\nfn main() {}\n# not a heading\n```\n");
            }
        }
        total += text.len();
        write(&root, &format!("{}-bench-{i}.md", KINDS[i % 9]), &text);
    }
    let mib = total as f64 / (1024.0 * 1024.0);
    assert!((5.5..=6.5).contains(&mib), "{mib} MiB");
    (root, mib)
}

const LIBRARY_ORIGIN: &str = "url: https://example.com/doc";

/// Writes `<root>/library/<corpus>/<name>.md` in the library-store format with a correct digest. `body` opens
/// with its `# <title>` line, so the title is line 7: the frontmatter takes lines 1 to 6.
pub fn library(root: &Path, corpus: &str, name: &str, body: &str) -> PathBuf {
    let digest = format!("sha256:{}", sha256(body));
    let text = format!(
        "---\nid: {}\nfetched: 2026-10-03\norigin: \"{LIBRARY_ORIGIN}\"\ndigest: {digest}\n---\n{body}",
        IDS[0]
    );
    let path = root.join("library").join(corpus).join(format!("{name}.md"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

/// Writes `<root>/library/<corpus>/guide.md`. Lines: the title is 6, `lead` is 8, and entry `i` is
/// `\n## <name>\n\n<prose>\n`, a heading on line 10 for the first and 4 lines further for each next one when its
/// prose is one line.
pub fn guide(root: &Path, corpus: &str, lead: &str, entries: &[(&str, &str)]) -> PathBuf {
    let mut text = format!(
        "---\nid: {}\ncreated: 2026-10-03T10:00-03:00\n---\n\n# {corpus}\n\n{lead}\n",
        IDS[1]
    );
    for (name, prose) in entries {
        text.push_str(&format!("\n## {name}\n\n{prose}\n"));
    }
    let path = root.join("library").join(corpus).join("guide.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn sha256(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A book: `chapters` level-2 sections of a paragraph and `parts` level-3 sections each, with a code fence in every
/// third chapter.
fn bench_book(seed: &mut u64, title: &str, chapters: usize, parts: usize) -> String {
    let mut text = format!("# {title}\n\n{}\n", paragraph(seed));
    for chapter in 0..chapters {
        text.push_str(&format!("\n## Chapter {chapter}\n\n{}\n", paragraph(seed)));
        for part in 0..parts {
            text.push_str(&format!("\n### Part {part}\n\n{}\n", paragraph(seed)));
        }
        if chapter % 3 == 0 {
            text.push_str("\n```\nfn main() {}\n# not a heading\n```\n");
        }
    }
    text
}

/// A catalog: `entries` short level-2 sections.
fn bench_catalog(seed: &mut u64, title: &str, entries: usize) -> String {
    let mut text = format!("# {title}\n\n{}\n", paragraph(seed));
    for entry in 0..entries {
        let words = 30 + (xorshift(seed) % 40) as usize;
        text.push_str(&format!(
            "\n## entry_{entry}\n\n{}\n",
            bench_text(seed, words)
        ));
    }
    text
}

/// A generated library of 8 corpora, about 14 MiB, under `<root>/library/`: two corpora of large books with three
/// heading levels, one catalog of hundreds of short sections beside a book, and five of many short sources, each
/// with a guide. Returns its size in MiB.
pub fn bench_library(root: &Path) -> f64 {
    let mut seed = 0xD1B5_4A32_D192_ED03u64;
    let mut total = 0usize;
    let mut put = |root: &Path, corpus: &str, name: &str, body: &str| {
        let path = library(root, corpus, name, body);
        total += std::fs::metadata(path).unwrap().len() as usize;
    };
    for corpus in ["books-a", "books-b"] {
        for book in 0..3 {
            let body = bench_book(&mut seed, &format!("Book {book}"), 265, 3);
            put(root, corpus, &format!("book-{book}"), &body);
        }
    }
    let catalog = bench_catalog(&mut seed, "Lints", 3800);
    put(root, "catalog", "lints", &catalog);
    let body = bench_book(&mut seed, "Handbook", 265, 3);
    put(root, "catalog", "handbook", &body);
    for corpus in ["short-a", "short-b", "short-c", "short-d", "short-e"] {
        for n in 0..215 {
            let body = bench_book(&mut seed, &format!("Page {n}"), 4, 2);
            put(root, corpus, &format!("page-{n}"), &body);
        }
    }
    for corpus in std::fs::read_dir(root.join("library")).unwrap() {
        let corpus = corpus.unwrap().file_name().into_string().unwrap();
        guide(
            root,
            &corpus,
            "Bench corpus.",
            &[("page-0", "A source about timeouts and decisions.")],
        );
        total += std::fs::metadata(root.join("library").join(&corpus).join("guide.md"))
            .unwrap()
            .len() as usize;
    }
    let mib = total as f64 / (1024.0 * 1024.0);
    assert!((13.5..=14.5).contains(&mib), "{mib} MiB");
    mib
}

/// Sets `path` to mode 000 and restores `mode` when dropped, so a failed assertion still lets the `TempDir` clean up.
#[cfg(unix)]
pub struct Locked {
    path: PathBuf,
    mode: u32,
}

#[cfg(unix)]
impl Locked {
    pub fn new(path: &Path, mode: u32) -> Locked {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        Locked {
            path: path.to_path_buf(),
            mode,
        }
    }
}

#[cfg(unix)]
impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.mode));
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Polls `read` every 50 ms until it returns `want`, for at most 40 seconds, so a test waits for a recording
/// without a fixed sleep.
pub fn poll_eq<T: PartialEq + std::fmt::Debug>(what: &str, mut read: impl FnMut() -> T, want: T) {
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    loop {
        let got = read();
        if got == want {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: wanted {want:?}, last saw {got:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Like `bilbo`, with stdout as bytes.
pub fn bilbo_bytes(cwd: &Path, env: &[(&str, &str)], args: &[&str]) -> (i32, Vec<u8>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_bilbo"))
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        output.stdout,
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// A running `bilbo watch`, killed when dropped, with its stderr lines collected by a thread.
pub struct Watcher {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
    reader: Option<JoinHandle<()>>,
}

impl Watcher {
    pub fn start(env: &[(&str, &str)]) -> Watcher {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bilbo"))
            .env_clear()
            .envs(env.iter().copied())
            .arg("watch")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                sink.lock().unwrap().push(line);
            }
        });
        Watcher {
            child,
            lines,
            reader: Some(reader),
        }
    }

    /// Starts a watcher on the store at `root` and waits for its watching line.
    pub fn on(root: &Path) -> Watcher {
        let watcher = Watcher::start(&[("BILBO_HOME", root.to_str().unwrap())]);
        watcher.wait_for(&format!("bilbo: watching {}", root.join("notes").display()));
        watcher
    }

    pub fn lines(&self) -> Vec<String> {
        let lines = self.lines.lock().unwrap().clone();
        for line in &lines {
            assert!(
                line.starts_with("bilbo: "),
                "stderr line without the prefix: {line:?}"
            );
        }
        lines
    }

    pub fn count(&self, needle: &str) -> usize {
        self.lines().iter().filter(|l| l.contains(needle)).count()
    }

    /// Waits until a stderr line contains `needle`.
    pub fn wait_for(&self, needle: &str) {
        self.wait_count(needle, 1);
    }

    pub fn wait_count(&self, needle: &str, n: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        while self.count(needle) < n {
            assert!(
                std::time::Instant::now() < deadline,
                "no stderr line with {needle:?} (wanted {n}); stderr: {:#?}",
                self.lines()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// A `bilbo relay` child on `127.0.0.1:0` with a clean environment, its stderr lines collected.
pub struct Relay {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
    reader: Option<JoinHandle<()>>,
    pub port: u16,
}

impl Relay {
    /// Starts `bilbo relay --data <data> --owner <owner>... --listen 127.0.0.1:0 <extra>` and waits for its startup
    /// line, which names the port.
    pub fn start(data: &Path, owners: &[&str], extra: &[&str]) -> Relay {
        Relay::start_with(data, owners, "127.0.0.1:0", extra, &[])
    }

    /// Like `start`, listening on `listen` (an address of this machine) with `env` as its whole environment.
    pub fn start_with(
        data: &Path,
        owners: &[&str],
        listen: &str,
        extra: &[&str],
        env: &[(&str, &str)],
    ) -> Relay {
        let mut args = vec![
            "relay".to_string(),
            "--data".into(),
            data.display().to_string(),
        ];
        for owner in owners {
            args.extend(["--owner".to_string(), owner.to_string()]);
        }
        args.extend(["--listen".to_string(), listen.to_string()]);
        args.extend(extra.iter().map(|a| a.to_string()));
        let mut child = Command::new(env!("CARGO_BIN_EXE_bilbo"))
            .env_clear()
            .envs(env.iter().copied())
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                sink.lock().unwrap().push(line);
            }
        });
        let mut relay = Relay {
            child,
            lines,
            reader: Some(reader),
            port: 0,
        };
        let host = listen.rsplit_once(':').map_or(listen, |(host, _)| host);
        let prefix = format!("bilbo: relay listening on http://{host}:");
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        relay.port = loop {
            if let Some(port) = relay
                .lines()
                .first()
                .and_then(|l| l.strip_prefix(prefix.as_str()))
                .and_then(|p| p.parse().ok())
            {
                break port;
            }
            assert!(
                relay.child.try_wait().unwrap().is_none(),
                "the relay exited; stderr: {:#?}",
                relay.lines()
            );
            assert!(
                std::time::Instant::now() < deadline,
                "no startup line; stderr: {:#?}",
                relay.lines()
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        relay
    }

    /// `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    /// Waits until a stderr line contains `needle`.
    pub fn wait_for(&self, needle: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        while !self.lines().iter().any(|l| l.contains(needle)) {
            assert!(
                std::time::Instant::now() < deadline,
                "no stderr line with {needle:?}; stderr: {:#?}",
                self.lines()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Sends SIGKILL and reaps the child.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.kill();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// `At` as the log writes it, `days` days before now.
pub fn days_ago(days: i64) -> String {
    let at = jiff::Zoned::now()
        .checked_sub(jiff::Span::new().days(days))
        .unwrap();
    at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// One version for `seed`.
pub struct Seed<'a> {
    pub file: &'a str,
    /// `None` is a deletion.
    pub text: Option<&'a [u8]>,
    pub event: &'a str,
    pub at: String,
    pub version: Option<String>,
}

impl<'a> Seed<'a> {
    pub fn new(file: &'a str, text: Option<&'a str>, event: &'a str, at: &str) -> Seed<'a> {
        Seed {
            file,
            text: text.map(str::as_bytes),
            event,
            at: at.to_string(),
            version: None,
        }
    }

    pub fn id(mut self, version: &str) -> Seed<'a> {
        self.version = Some(version.to_string());
        self
    }
}

/// Writes a note's log and the blobs it names under `<root>/.bilbo/history/`, one version after another, and returns
/// the version ids. The ids are made up (the reader only checks that they are hex), each one's parent the one before.
pub fn seed(root: &Path, note_id: &str, versions: &[Seed]) -> Vec<String> {
    let history = root.join(".bilbo/history");
    std::fs::create_dir_all(history.join("notes")).unwrap();
    let mut ids: Vec<String> = Vec::new();
    let mut log = String::new();
    for (n, seed) in versions.iter().enumerate() {
        let version = seed
            .version
            .clone()
            .unwrap_or_else(|| sha256_hex(format!("{note_id}:{n}").as_bytes()));
        let blob = match seed.text {
            Some(text) => {
                let blob = sha256_hex(text);
                let dir = history.join("blobs").join(&blob[..2]);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(&blob[2..]), text).unwrap();
                blob
            }
            None => "deleted".to_string(),
        };
        let parents: Vec<&String> = ids.last().into_iter().collect();
        log.push_str(
            &serde_json::json!({
                "version": version,
                "parents": parents,
                "file": seed.file,
                "blob": blob,
                "event": seed.event,
                "at": seed.at,
            })
            .to_string(),
        );
        log.push('\n');
        ids.push(version);
    }
    std::fs::write(history.join("notes").join(format!("{note_id}.jsonl")), log).unwrap();
    ids
}

/// The CPU time a process has used, in seconds: `/proc/<pid>/stat` on Linux, `ps -o cputime=` elsewhere. `None`
/// where neither answers, as in the macOS build sandbox, which hides `ps`.
pub fn cpu_seconds(pid: u32) -> Option<f64> {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        let after = &stat[stat.rfind(')')? + 2..];
        let fields: Vec<&str> = after.split(' ').collect();
        let ticks = fields[11].parse::<f64>().ok()? + fields[12].parse::<f64>().ok()?;
        return Some(ticks / 100.0);
    }
    let out = Command::new("ps")
        .args(["-o", "cputime=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    text.trim().split(':').try_fold(0.0, |total, part| {
        Some(total * 60.0 + part.parse::<f64>().ok()?)
    })
}

/// `text`, a note, with `scope: <scope>` as its last frontmatter line.
pub fn in_scope(text: &str, scope: &str) -> String {
    text.replacen("\n---\n\n#", &format!("\nscope: {scope}\n---\n\n#"), 1)
}

/// Scope tests: HOME, the working folders, the store and the config share one temporary tree.
pub struct Scoped {
    pub home: PathBuf,
    pub root: PathBuf,
    pub config: PathBuf,
}

/// `<dir>/home`, `<dir>/store` and a config holding `lines`; `~/` in a scope path means `<dir>/home`.
pub fn scoped(dir: &TempDir, lines: &[&str]) -> Scoped {
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    Scoped {
        home,
        root: dir.path().join("store"),
        config: config(dir, lines),
    }
}

/// Runs bilbo in `cwd` with `scoped`'s HOME, store and config.
pub fn bilbo_scoped(scoped: &Scoped, cwd: &Path, args: &[&str]) -> Run {
    bilbo(
        cwd,
        &[
            ("HOME", scoped.home.to_str().unwrap()),
            ("BILBO_HOME", scoped.root.to_str().unwrap()),
            ("BILBO_CONFIG", scoped.config.to_str().unwrap()),
        ],
        args,
    )
}
