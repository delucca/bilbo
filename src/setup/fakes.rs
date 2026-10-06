//! Test helpers that several of setup's modules use.

use std::path::{Path, PathBuf};

use super::facts::{ConfigState, Facts, TimerFacts};
use super::local::Outside;
use super::plan::{ConfigPlan, EmbedderPlan, KeyPlan, Plan, PluginPlan, TimerPlan};
use super::syncing::SyncPlan;
use crate::host::{agents, model};
use crate::shared::config::{self, Embedder};

pub fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

pub fn embedder(model: &str) -> Embedder {
    Embedder {
        url: "http://127.0.0.1:8081".into(),
        model: model.into(),
        token: None,
        query_prefix: config::default_query_prefix(model).into(),
        min_similarity: config::DEFAULT_MIN_SIMILARITY,
    }
}

pub fn plan(config: ConfigPlan, embedder: Option<Embedder>, check: EmbedderPlan) -> Plan {
    Plan {
        notes: "/r/notes".into(),
        store_exists: false,
        config_path: "/c/bilbo/config".into(),
        config,
        embedder,
        kept: Vec::new(),
        key: KeyPlan::NoEmbedder,
        pasted: None,
        check,
        local: None,
        local_lines: None,
        unused_service: None,
        source: agents::Source::GitHub {
            repo: "delucca/bilbo".into(),
            git_ref: "v1.2.3".into(),
        },
        claude: PluginPlan::Skipped("--no-plugin"),
        codex: PluginPlan::Skipped("--no-plugin"),
        timer: TimerPlan::Skipped("no embedder"),
        watch: TimerPlan::Skipped("--no-watch"),
        sync: SyncPlan::default(),
    }
}

/// Writes an executable through a child `sh`, so this process never holds it open for
/// writing: a test forking at that moment would inherit the descriptor, and running the
/// file would fail with "Text file busy" on Linux (rust-lang/rust#114554).
pub fn write_script(path: &Path, text: &str) {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bilbo-setup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn seen(dir: &Path, existing: Option<Embedder>, config: ConfigState) -> Facts {
    Facts {
        root: dir.to_path_buf(),
        notes: dir.join("notes"),
        config_path: dir.join("cfg/config"),
        config,
        existing,
        kept: Vec::new(),
        config_empty: false,
        token_path: dir.join("cfg/token"),
        exe: "/bin/bilbo".into(),
        source: agents::Source::GitHub {
            repo: "delucca/bilbo".into(),
            git_ref: "v1.2.3".into(),
        },
        claude: None,
        codex: None,
        timer: TimerFacts {
            platform: None,
            home: None,
            config_home: None,
            state_dir: None,
            tool: None,
            locations: Ok(Vec::new()),
        },
        llama_server: None,
        model: None,
        keys: None,
        scopes: Vec::new(),
        agent: false,
        host: None,
    }
}

/// A manager that exits 0 and, for systemd, reports a live user session.
pub const LIVE_MANAGER: &str =
    "#!/bin/sh\ncase \"$*\" in\n\"--user is-system-running\") echo running;;\nesac\nexit 0\n";

/// An `Outside` that answers from a script and records its calls.
pub struct Script {
    pub fetch: Result<(), String>,
    pub ready: Result<(), String>,
    pub check: Result<usize, String>,
    /// Check answers that fail before `check` takes over, first to last.
    pub failures: Vec<String>,
    pub calls: Vec<&'static str>,
}

impl Script {
    pub fn working() -> Script {
        Script {
            fetch: Ok(()),
            ready: Ok(()),
            check: Ok(1024),
            failures: Vec::new(),
            calls: Vec::new(),
        }
    }
}

impl Outside for Script {
    fn fetch(&mut self, path: &Path, _: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        self.calls.push("fetch");
        if self.fetch.is_ok() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::File::create(path)
                .unwrap()
                .set_len(model::PINNED.size)
                .unwrap();
        }
        self.fetch.clone()
    }
    fn ready(&mut self, _: &str) -> Result<(), String> {
        self.calls.push("ready");
        self.ready.clone()
    }
    fn check(&mut self, _: &Embedder) -> Result<usize, String> {
        self.calls.push("check");
        if !self.failures.is_empty() {
            return Err(self.failures.remove(0));
        }
        self.check.clone()
    }
    fn listening(&mut self, _: u16) -> bool {
        false
    }
}

/// A relay on loopback that admits the owners with these fingerprints, on `port` or any free one; its data is under
/// the scratch folder `name`.
pub fn relay(name: &str, owners: &[String], port: u16) -> crate::relay::Running {
    relay_logging(name, owners, port, Default::default())
}

/// `relay`, with every line the relay logs pushed to `lines`.
pub fn relay_logging(
    name: &str,
    owners: &[String],
    port: u16,
    lines: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> crate::relay::Running {
    use crate::relay::{self, Flags};
    let data = scratch(name).join("data");
    relay::start(
        Flags {
            data,
            owners: owners.to_vec(),
            listen: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            max_scopes: 16,
            max_scope_mb: 1024,
            max_object_mb: 16,
        },
        relay::system_clock(),
        std::sync::Arc::new(move |line: &str| lines.lock().unwrap().push(line.to_string())),
    )
    .unwrap()
}

/// A loopback server that answers every request with a status and an empty JSON object, and keeps each request's
/// head.
pub struct Answering {
    port: u16,
    seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Answering {
    pub fn start(status: u16) -> Answering {
        use std::io::{BufRead, BufReader, Write};
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (record, halt) = (seen.clone(), stop.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                let mut head = String::new();
                let mut lines = BufReader::new(&stream);
                loop {
                    let mut line = String::new();
                    if lines.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                record.lock().unwrap().push(head);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
            }
        });
        Answering { port, seen, stop }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The request lines and headers received so far.
    pub fn heads(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Answering {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}
