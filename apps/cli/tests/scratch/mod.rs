use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
