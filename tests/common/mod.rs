#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

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
