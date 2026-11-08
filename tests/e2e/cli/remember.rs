use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::bilbo;

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn base() -> PathBuf {
    std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(test: &str) -> Scratch {
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
        Scratch { dir }
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_str().expect("UTF-8").to_owned()
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("HOME", self.path("home")),
            ("BILBO_HOME", self.path("store")),
            ("XDG_STATE_HOME", self.path("state")),
            ("XDG_CACHE_HOME", self.path("cache")),
            ("BILBO_CONFIG", self.path("config/config")),
        ]
    }

    fn run(&self, args: &[&str]) -> super::Run {
        let env = self.env();
        let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        bilbo(args, &pairs)
    }

    fn write(&self, name: &str, bytes: &str) {
        let path = self.dir.join("notes").join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("folders");
        fs::write(path, bytes).expect("write");
    }

    fn segments(&self) -> usize {
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

    fn notes_state(&self) -> Vec<(PathBuf, Vec<u8>)> {
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

#[test]
fn remembering_a_folder_twice_keeps_everything_once() {
    let scratch = Scratch::new("twice");
    scratch.write(
        "a.md",
        "---\nid: 01JC5Q3V7XKM4P8R2T6W9YB0DE\n---\n# A\nline\n",
    );
    scratch.write("b.md", "no final newline");
    scratch.write("sub/c.md", "c\n");
    scratch.write(".hidden.md", "h\n");
    scratch.write(".git/d.md", "d\n");
    scratch.write("x.txt", "x\n");
    let before = scratch.notes_state();
    let notes = scratch.path("notes");

    let first = scratch.run(&["remember", &notes]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert_eq!(first.stderr, "");
    let mut lines = first.stdout.lines();
    assert_eq!(
        lines.next(),
        Some("new 3, appended 0, new generation 0, unchanged 0, refused 0")
    );
    let totals = lines.next().expect("a second line");
    assert!(
        totals.starts_with("records 7, raw bytes 66, stored bytes "),
        "{totals}"
    );
    assert!(!totals.ends_with(" 0"), "{totals}");
    assert_eq!(scratch.segments(), 1);
    let open_seg = fs::read(scratch.dir.join("state/bilbo/open.seg")).expect("open.seg");
    assert_eq!(open_seg.len(), 12);
    assert!(scratch.dir.join("cache/bilbo/catalog.sqlite").exists());

    let second = scratch.run(&["remember", &notes]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert_eq!(
        second.stdout,
        "new 0, appended 0, new generation 0, unchanged 3, refused 0\nrecords 0, raw bytes 0, stored bytes 0\n"
    );
    assert_eq!(scratch.segments(), 1);
    assert_eq!(scratch.notes_state(), before);
}

#[test]
fn remember_usage_errors_name_the_problem_and_touch_no_store() {
    const USAGE: &str = "bilbo: usage: bilbo remember <file|folder> [--kind <kind>] [--project <project>]\nbilbo: see 'bilbo remember --help'\n";
    let scratch = Scratch::new("usage");
    let cases: [(&[&str], &str); 7] = [
        (&["remember"], "bilbo: missing <file|folder>\n"),
        (&["remember", ""], "bilbo: <file|folder> needs a value\n"),
        (
            &["remember", "x", "--kind"],
            "bilbo: --kind needs a value\n",
        ),
        (
            &["remember", "x", "--kind="],
            "bilbo: --kind needs a value\n",
        ),
        (
            &["remember", "x", "--project="],
            "bilbo: --project needs a value\n",
        ),
        (&["remember", "a", "b"], "bilbo: unexpected argument 'b'\n"),
        (
            &["remember", "x", "--kind", "nope"],
            "bilbo: unknown kind 'nope'; kinds: markdown\n",
        ),
    ];
    for (args, reason) in cases {
        let run = scratch.run(args);
        assert_eq!(run.code, 2, "{args:?}");
        assert_eq!(run.stdout, "", "{args:?}");
        assert_eq!(run.stderr, format!("{reason}{USAGE}"), "{args:?}");
    }
    for folder in ["store", "state/bilbo", "cache/bilbo"] {
        assert!(!scratch.dir.join(folder).exists(), "{folder}");
    }
}
