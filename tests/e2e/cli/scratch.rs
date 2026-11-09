use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use super::{Run, bilbo};

static COUNTER: AtomicU64 = AtomicU64::new(0);

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
        fs::write(dir.join("config/config.toml"), "").expect("an empty config file");
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
