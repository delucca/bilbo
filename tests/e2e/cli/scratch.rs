use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use super::{Run, bilbo, binary};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Probe(PathBuf);

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn closed(text: &str) -> String {
    let dir = base()
        .canonicalize()
        .expect("canonical scratch base")
        .join(format!(
            "closed-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
    let probe = Probe(dir);
    let dir = &probe.0;
    fs::create_dir_all(dir.join("home")).expect("home");
    fs::write(dir.join("config.toml"), text).expect("a config file");
    let path = |name: &str| dir.join(name).to_str().expect("UTF-8").to_owned();
    let env = [
        ("HOME", path("home")),
        ("BILBO_HOME", path("store")),
        ("XDG_STATE_HOME", path("state")),
        ("XDG_CACHE_HOME", path("cache")),
        ("BILBO_CONFIG", path("config.toml")),
    ];
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let read = bilbo(&["config"], &pairs);
    let mut whole = String::new();
    for provider in ["anthropic", "openai"] {
        let key = format!("llm.{provider}.api.endpoint = ");
        let own = if read.code == 0 {
            read.stdout
                .lines()
                .any(|line| line.starts_with(&key) && !line.ends_with("(default)"))
        } else {
            text.contains(key.trim_end_matches(" = "))
        };
        if !own {
            whole.push_str(&format!(
                "llm.{provider}.api.endpoint = \"http://127.0.0.1:9/\"\n"
            ));
        }
    }
    whole.push_str(text);
    whole
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn base() -> PathBuf {
    std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

pub(crate) struct Scratch {
    pub(crate) dir: PathBuf,
}

impl Scratch {
    pub(crate) fn new(test: &str) -> Scratch {
        let dir = base()
            .canonicalize()
            .expect("canonical scratch base")
            .join(format!(
                "{test}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("home")).expect("home");
        fs::create_dir_all(dir.join("notes")).expect("notes");
        fs::create_dir_all(dir.join("config")).expect("config");
        fs::write(dir.join("config/config.toml"), closed("")).expect("a config file");
        Scratch { dir }
    }

    pub(crate) fn path(&self, name: &str) -> String {
        self.dir.join(name).to_str().expect("UTF-8").to_owned()
    }

    pub(crate) fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("HOME", self.path("home")),
            ("BILBO_HOME", self.path("store")),
            ("XDG_STATE_HOME", self.path("state")),
            ("XDG_CACHE_HOME", self.path("cache")),
            ("BILBO_CONFIG", self.path("config/config.toml")),
        ]
    }

    pub(crate) fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, &[])
    }

    /// A run with `extra` variables set after the scratch ones, so they win.
    pub(crate) fn run_with(&self, args: &[&str], extra: &[(&'static str, &str)]) -> Run {
        let mut env = self.env();
        env.extend(
            extra
                .iter()
                .map(|(name, value)| (*name, (*value).to_owned())),
        );
        let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        bilbo(args, &pairs)
    }

    pub(crate) fn run_within(
        &self,
        args: &[&str],
        extra: &[(&str, &str)],
        folder: Option<&Path>,
        limit: Duration,
        input: Option<&[u8]>,
    ) -> Run {
        let stdin = input.map_or_else(Stdio::null, |bytes| {
            let path = self
                .dir
                .join(format!("stdin-{}", COUNTER.fetch_add(1, Ordering::Relaxed)));
            fs::write(&path, bytes).expect("the input is written");
            assert_eq!(fs::read(&path).expect("the input is read back"), bytes);
            Stdio::from(fs::File::open(&path).expect("the input opens"))
        });
        let mut command = Command::new(fs::canonicalize(binary()).expect("the binary"));
        command
            .env_clear()
            .envs(self.env())
            .envs(extra.iter().copied())
            .args(args)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(folder) = folder {
            command.current_dir(folder);
        }
        let mut child = command.spawn().expect("bilbo starts");
        let started = Instant::now();
        while child.try_wait().expect("wait").is_none() {
            if started.elapsed() > limit {
                let _ = child.kill();
                let _ = child.wait();
                panic!("bilbo {args:?} did not exit within {limit:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().expect("output");
        Run {
            code: output.status.code().expect("an exit code"),
            stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
            stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        }
    }

    pub(crate) fn run_with_input(&self, args: &[&str], input: &[u8]) -> Run {
        self.piped(args, input, None, false)
    }

    pub(crate) fn run_with_input_in(&self, args: &[&str], input: &[u8], folder: &Path) -> Run {
        self.piped(args, input, Some(folder), false)
    }

    pub(crate) fn run_leaving_input_unread_in(
        &self,
        args: &[&str],
        input: &[u8],
        folder: &Path,
    ) -> Run {
        self.piped(args, input, Some(folder), true)
    }

    fn piped(&self, args: &[&str], input: &[u8], folder: Option<&Path>, unread: bool) -> Run {
        let mut command = Command::new(fs::canonicalize(binary()).expect("the binary"));
        command.env_clear().envs(self.env());
        if let Some(folder) = folder {
            command.current_dir(folder);
        }
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("bilbo runs");
        let written = child.stdin.take().expect("a pipe").write_all(input);
        if let Err(error) = written {
            assert!(
                unread && error.kind() == std::io::ErrorKind::BrokenPipe,
                "the bytes are not written: {error}"
            );
        }
        let output = child.wait_with_output().expect("bilbo ends");
        Run {
            code: output.status.code().expect("an exit code"),
            stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
            stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        }
    }

    /// A run without `BILBO_HOME`, so the root comes from the file or the default.
    pub(crate) fn run_without_home(&self, args: &[&str]) -> Run {
        self.run_without(args, "BILBO_HOME")
    }

    /// A run with the scratch environment but for the variable `dropped`.
    pub(crate) fn run_without(&self, args: &[&str], dropped: &str) -> Run {
        let env = self.env();
        let pairs: Vec<(&str, &str)> = env
            .iter()
            .filter(|(name, _)| *name != dropped)
            .map(|(k, v)| (*k, v.as_str()))
            .collect();
        bilbo(args, &pairs)
    }

    /// Writes the config file every run names through `BILBO_CONFIG`, and returns its path.
    pub(crate) fn config(&self, text: &str) -> String {
        let path = self.path("config/config.toml");
        fs::write(&path, text).expect("write the config file");
        path
    }

    pub(crate) fn write(&self, name: &str, bytes: &str) {
        let path = self.dir.join("notes").join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("folders");
        fs::write(path, bytes).expect("write");
    }

    /// Backdates the age record so the next close weighs it against `store.seal_after`.
    pub(crate) fn backdate(&self, minutes: u64) {
        let at = SystemTime::now() - Duration::from_secs(minutes * 60);
        let millis = at
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after 1970")
            .as_millis();
        fs::write(
            self.dir.join("state/bilbo/open.since"),
            format!("{millis}\n"),
        )
        .expect("backdate");
    }

    pub(crate) fn segments(&self) -> usize {
        fs::read_dir(self.dir.join("store"))
            .expect("the store")
            .filter(|e| {
                e.as_ref()
                    .expect("entry")
                    .path()
                    .extension()
                    .is_some_and(|x| x == "seg")
            })
            .count()
    }

    pub(crate) fn notes_state(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![self.dir.join("notes")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir).expect("list") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let bytes = fs::read(&path).expect("read");
                    files.push((path, bytes));
                }
            }
        }
        files.sort();
        files
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub(crate) const UUID: &str = "8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b";
pub(crate) const SESSION: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
);
pub(crate) const MAIN: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
pub(crate) const CLAUDE: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
);
pub(crate) const SUBAGENT: &[u8] =
    include_bytes!("../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.jsonl");
pub(crate) const NOTE: &str =
    include_str!("../../../libs/entries/rust/fixtures/markdown/sections.md");

pub(crate) fn remembered(test: &str) -> Scratch {
    let scratch = Scratch::new(test);
    let session = scratch.dir.join(format!(
        "home/.claude/projects/-home-me-src-garden/{UUID}.jsonl"
    ));
    fs::create_dir_all(session.parent().expect("a parent")).expect("folders");
    fs::write(&session, SESSION).expect("a session");
    scratch.write("sections.md", NOTE);
    for target in [
        session.to_str().expect("UTF-8").to_owned(),
        scratch.path("notes"),
    ] {
        let run = scratch.run(&["remember", &target]);
        assert_eq!(run.code, 0, "{}", run.stderr);
    }
    scratch
}
