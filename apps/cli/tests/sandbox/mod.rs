//! A scratch folder per test and an in-process `bilbo remember` pointed at it.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::Env;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;
use bilbo_store::layout::Layout;

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn base() -> PathBuf {
    std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

pub fn scratch_dir(test: &str) -> PathBuf {
    let base = base().canonicalize().expect("canonical scratch base");
    base.join(format!(
        "{test}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

pub fn vars(dir: &Path) -> HashMap<String, OsString> {
    [
        ("HOME", dir.join("home")),
        ("BILBO_HOME", dir.join("store")),
        ("XDG_STATE_HOME", dir.join("state")),
        ("XDG_CACHE_HOME", dir.join("cache")),
        ("BILBO_CONFIG", dir.join("config/config.toml")),
    ]
    .into_iter()
    .map(|(name, path)| (name.to_owned(), path.into_os_string()))
    .collect()
}

pub fn layout(vars: &HashMap<String, OsString>) -> Layout {
    settings::load(
        &|name| vars.get(name).cloned(),
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
    )
    .expect("settings")
    .layout()
}

pub struct Run {
    pub code: u8,
    pub stdout: String,
    pub stderr: Vec<String>,
    pub notices: Vec<String>,
}

/// Notices also go to `notices`, for a test that waits on one.
pub fn run(
    vars: HashMap<String, OsString>,
    args: &[&str],
    stdin: &[u8],
    notices: Option<Sender<String>>,
) -> Run {
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let mut env = Env {
        var: Box::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::Cursor::new(stdin.to_vec())),
        notice: Box::new(move |message| {
            let lines = message.plain_lines();
            if let Some(notices) = &notices {
                notices.send(lines.join("\n")).ok();
            }
            sink.lock().expect("notices").extend(lines);
        }),
    };
    let argv: Vec<OsString> = ["bilbo", "remember"]
        .into_iter()
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let settings = settings::load(&*env.var, &|path| fs::read(path), &bilbo_cli::kind_keys())
        .map_err(Failure::from);
    let (code, stdout, stderr) = match bilbo_cli::run(&argv, &mut env, settings) {
        Ok(outcome) => (
            outcome.exit_code(),
            outcome.stdout(&View::PLAIN),
            outcome.stderr_lines(&View::PLAIN),
        ),
        Err(failure) => (
            failure.exit_code(),
            String::new(),
            failure.stderr_lines(&View::PLAIN),
        ),
    };
    let notices = said.lock().expect("notices").clone();
    Run {
        code,
        stdout,
        stderr,
        notices,
    }
}

/// Backdates the age record so the next close is due to seal.
pub fn backdate(state: &Path, minutes: u64) {
    let at = SystemTime::now() - Duration::from_secs(minutes * 60);
    let millis = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_millis();
    fs::write(state.join("open.since"), format!("{millis}\n")).expect("backdate");
}

pub fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
    fs::write(path, bytes).expect("write");
}

pub fn text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 path")
}

pub fn counts(new: u64, appended: u64, generation: u64, unchanged: u64, refused: u64) -> String {
    format!(
        "new {new}, appended {appended}, new generation {generation}, unchanged {unchanged}, refused {refused}"
    )
}
