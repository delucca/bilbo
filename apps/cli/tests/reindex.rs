//! This file and `lib.rs` are the only ones that name the index's crate: a test of what the index holds has to read it.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::Env;
use bilbo_cli::console::indexing::Indexing;
use bilbo_cli::console::message::Message;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings;
use bilbo_cli::console::view::View;
use bilbo_kinds::reader::{Passage, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_search::analyze::Omit;
use bilbo_search::bm25::Bm25;
use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats, millis};
use bilbo_search::lookup::Found;
use bilbo_search::position::Position;
use bilbo_search::rank::{Candidate, Query, Ranker};
use bilbo_search::tokenizer::Tokenizer;
use bilbo_store::entries::{Entries, Generation, Origin};
use bilbo_store::id::Id;
use bilbo_store::segments::{Options, Store};
use bilbo_tantivy::{Committed, Row, Tantivy};

const SESSIONS: [(&str, &[u8]); 4] = [
    (
        "claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
        ),
    ),
    (
        "claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03/subagents/agent-ad63c2b3c9e7a4b3d.jsonl",
        include_bytes!("../../../libs/rust/kinds/session/fixtures/agent-ad63c2b3c9e7a4b3d.jsonl"),
    ),
    (
        "claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
        ),
    ),
    (
        "codex/2026/11/07/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
        ),
    ),
];
const NOTES: [(&str, &[u8]); 2] = [
    (
        "sections.md",
        include_bytes!("../../../libs/rust/kinds/markdown/fixtures/sections.md"),
    ),
    (
        "bare.md",
        include_bytes!("../../../libs/rust/kinds/markdown/fixtures/no-frontmatter.md"),
    ),
];
const QUERIES: [&str; 5] = [
    "garden",
    "carrots and beans",
    "watering in the morning",
    "session",
    "heading",
];
const WAIT: Duration = Duration::from_secs(20);

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Loading the tokenizer costs a second in a test build: the tests that are not about the loader
/// share one for the whole binary.
static QWEN: LazyLock<bilbo_qwen::Qwen> =
    LazyLock::new(|| bilbo_qwen::Qwen::load().expect("the tokenizer"));

const WORKERS: &str = "2";

fn options() -> bilbo_tantivy::Options {
    let search = bilbo_search::keys::defaults();
    bilbo_tantivy::Options {
        memory: search.memory(),
        merge_deleted_share: search.merge_deleted_share(),
    }
}

fn rank(cache: &Path, text: &str, limit: usize) -> Vec<Candidate> {
    let keys = bilbo_search::recall::keys::defaults();
    let index = Committed::at(bilbo_tantivy::folder(cache));
    Bm25 {
        index: &index,
        k1: keys.bm25_k1.value,
        b: keys.bm25_b.value,
        omit: Omit::default(),
        batch: keys.read_batch.value,
    }
    .rank(
        &Query {
            text: text.to_owned(),
            kinds: Vec::new(),
        },
        limit,
    )
    .expect("a search")
}

fn disk(cache: &Path) -> Box<dyn Index> {
    bilbo_cli::index(cache, &bilbo_search::keys::defaults())
}

struct Shared;

impl Tokens for Shared {
    fn ends(&self, text: &str) -> Vec<usize> {
        QWEN.ends(text)
    }

    fn count(&self, text: &str) -> usize {
        QWEN.count(text)
    }

    fn prefix(&self, text: &str, budget: usize) -> usize {
        QWEN.prefix(text, budget)
    }
}

fn shared_tokenizer() -> Result<Tokenizer, Failure> {
    Ok(Tokenizer {
        tokens: Box::new(Shared),
        id: bilbo_qwen::ID,
    })
}

fn tune(mut indexing: Indexing<'_>) -> Indexing<'_> {
    indexing.tokenizer = &shared_tokenizer;
    indexing
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn base() -> PathBuf {
    std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

struct Run {
    code: u8,
    stdout: String,
    stderr: Vec<String>,
}

struct Sandbox {
    dir: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = base().canonicalize().expect("a scratch base").join(format!(
            "{test}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        for folder in ["home", "config", "sessions", "notes"] {
            fs::create_dir_all(dir.join(folder)).expect("create scratch");
        }
        fs::write(dir.join("config/config.toml"), "").expect("an empty config file");
        Sandbox { dir }
    }

    fn filled(test: &str) -> Sandbox {
        let sandbox = Sandbox::new(test);
        for (name, bytes) in SESSIONS {
            sandbox.write(&format!("sessions/{name}"), bytes);
        }
        for (name, bytes) in NOTES {
            sandbox.write(&format!("notes/{name}"), bytes);
        }
        for folder in ["sessions", "notes"] {
            let run = sandbox.run(&["remember", &sandbox.path(folder)]);
            assert_eq!(run.code, 0, "{:?}", run.stderr);
        }
        sandbox
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_str().expect("UTF-8").to_owned()
    }

    /// Written an hour ago, so `remember` trusts its size and time enough to record them.
    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.dir.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(&path, bytes).expect("write");
        let file = fs::File::options().write(true).open(&path).expect("open");
        file.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .expect("backdate");
    }

    fn vars(&self) -> HashMap<String, OsString> {
        let mut vars: HashMap<String, OsString> = [
            ("HOME", "home"),
            ("BILBO_HOME", "store"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("BILBO_CONFIG", "config/config.toml"),
        ]
        .into_iter()
        .map(|(name, folder)| (name.to_owned(), self.dir.join(folder).into_os_string()))
        .collect();
        vars.insert("BILBO_SEARCH_WORKERS".to_owned(), OsString::from(WORKERS));
        vars
    }

    fn cache(&self) -> PathBuf {
        self.dir.join("cache/bilbo")
    }

    fn index(&self) -> Tantivy {
        Tantivy::at(bilbo_tantivy::folder(&self.cache()), options())
    }

    fn rank(&self, text: &str, limit: usize) -> Vec<Candidate> {
        rank(&self.cache(), text, limit)
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, &[], None)
    }

    fn run_with(
        &self,
        args: &[&str],
        extra: &[(&str, &str)],
        notices: Option<Sender<String>>,
    ) -> Run {
        let mut vars = self.vars();
        vars.extend(
            extra
                .iter()
                .map(|(name, value)| ((*name).to_owned(), OsString::from(value))),
        );
        bilbo(vars, args, notices)
    }

    fn payload(&self) -> String {
        let meta = fs::read_to_string(bilbo_tantivy::folder(&self.cache()).join("meta.json"))
            .expect("meta.json");
        let meta: serde_json::Value = serde_json::from_str(&meta).expect("JSON");
        meta["payload"].as_str().expect("a payload").to_owned()
    }

    fn seq(&self) -> u64 {
        let payload = self.payload();
        payload
            .lines()
            .find_map(|line| line.strip_prefix("seq "))
            .expect("a seq line")
            .parse()
            .expect("a number")
    }

    fn matching(&self) -> (Vec<String>, Vec<Vec<String>>) {
        let index = self.index();
        let sorted = |rows: Vec<String>| {
            let mut rows = rows;
            rows.sort();
            rows
        };
        let hits = QUERIES
            .iter()
            .map(|query| {
                sorted(
                    self.rank(query, 50)
                        .into_iter()
                        .map(|candidate| format!("{:?}", candidate.found))
                        .collect(),
                )
            })
            .collect();
        let rows = index
            .documents()
            .expect("documents")
            .iter()
            .map(|row| format!("{row:?}"))
            .collect();
        (sorted(rows), hits)
    }

    fn remember_in_process(
        &self,
        target: &str,
        index: &dyn Fn(&Path) -> Box<dyn Index>,
        loads: &AtomicU64,
    ) -> Run {
        let load = || {
            loads.fetch_add(1, Ordering::Relaxed);
            bilbo_cli::tokenizer()
        };
        self.remember_with(target, index, &load, false, &Arc::default())
    }

    fn remember_with(
        &self,
        target: &str,
        index: &dyn Fn(&Path) -> Box<dyn Index>,
        load: &dyn Fn() -> Result<Tokenizer, Failure>,
        panicky: bool,
        said: &Arc<Mutex<Vec<String>>>,
    ) -> Run {
        self.remember_with_vars(target, &[], index, load, panicky, said)
    }

    fn remember_with_vars(
        &self,
        target: &str,
        extra: &[(&str, &str)],
        index: &dyn Fn(&Path) -> Box<dyn Index>,
        load: &dyn Fn() -> Result<Tokenizer, Failure>,
        panicky: bool,
        said: &Arc<Mutex<Vec<String>>>,
    ) -> Run {
        let sink = Arc::clone(said);
        let mut vars = self.vars();
        vars.extend(
            extra
                .iter()
                .map(|(name, value)| ((*name).to_owned(), OsString::from(value))),
        );
        let mut env = Env {
            var: Box::new(move |name| vars.get(name).cloned()),
            stdin: Box::new(io::empty()),
            notice: Box::new(move |message| {
                sink.lock().expect("notices").extend(message.plain_lines());
            }),
        };
        let settings = settings::load(&*env.var, &|path| fs::read(path), &bilbo_cli::kind_keys())
            .expect("settings");
        let mut readers = bilbo_cli::readers(&settings).expect("the readers");
        if panicky {
            readers = readers
                .into_iter()
                .map(|reader| Box::new(Poisoned(reader)) as Box<dyn Reader>)
                .collect();
        }
        let rendering = bilbo_cli::rendering(&settings).expect("the rendering");
        let indexing = bilbo_cli::indexing(&settings, &readers, &rendering, load, index);
        let args = bilbo_cli::remember::Args {
            target: Some(PathBuf::from(target)),
            kind: None,
            follow: false,
        };
        let outcome = bilbo_cli::remember::run(
            &bilbo_cli::bilbo(),
            args,
            &mut env,
            &settings,
            &bilbo_cli::kinds(&settings).expect("the kinds"),
            &indexing,
        )
        .expect("the remember");
        let mut stderr = said.lock().expect("notices").clone();
        stderr.extend(outcome.stderr_lines(&View::PLAIN));
        Run {
            code: outcome.exit_code(),
            stdout: outcome.stdout(&View::PLAIN),
            stderr,
        }
    }

    fn reindex(&self) -> Run {
        let run = self.run(&["reindex"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        run
    }

    fn store(&self) -> Store {
        let layout = settings::load(
            &|name| self.vars().get(name).cloned(),
            &|path| fs::read(path),
            &bilbo_cli::kind_keys(),
        )
        .expect("settings")
        .layout();
        Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("open the store")
    }

    fn held(&self) -> (Vec<Row>, Vec<Vec<(u32, Found)>>) {
        let index = self.index();
        let hits = QUERIES
            .iter()
            .map(|query| {
                self.rank(query, 20)
                    .into_iter()
                    .map(|candidate| (candidate.score.to_bits(), candidate.found))
                    .collect()
            })
            .collect();
        (index.documents().expect("the documents"), hits)
    }
}

struct Poisoned(Box<dyn Reader>);

impl Reader for Poisoned {
    fn kind(&self) -> &'static str {
        self.0.kind()
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        assert!(
            !records
                .iter()
                .any(|record| record.windows(6).any(|window| window == b"POISON")),
            "a reader that fails on one record"
        );
        self.0.passages(records)
    }

    fn appends_only(&self) -> bool {
        self.0.appends_only()
    }
}

fn bilbo(vars: HashMap<String, OsString>, args: &[&str], notices: Option<Sender<String>>) -> Run {
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let mut env = Env {
        var: Box::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        notice: Box::new(move |message| {
            let lines = message.plain_lines();
            if let Some(notices) = &notices {
                notices.send(lines.join("\n")).ok();
            }
            sink.lock().expect("notices").extend(lines);
        }),
    };
    let argv: Vec<OsString> = std::iter::once("bilbo")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let settings = settings::load(&*env.var, &|path| fs::read(path), &bilbo_cli::kind_keys())
        .map_err(Failure::from);
    let (code, stdout, lines) =
        match bilbo_cli::run_tuned(&bilbo_cli::bilbo(), &argv, &mut env, settings, &tune) {
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
    let mut stderr = said.lock().expect("notices").clone();
    stderr.extend(lines);
    Run {
        code,
        stdout,
        stderr,
    }
}

/// `entries`, `chunks`, `unread`, `failed` and the index's bytes, from the two lines of the output.
fn numbers(stdout: &str) -> [u64; 5] {
    let found: Vec<u64> = stdout
        .split(|c: char| !c.is_ascii_digit())
        .filter(|digits| !digits.is_empty())
        .map(|digits| digits.parse().expect("a number"))
        .collect();
    assert_eq!(found.len(), 6, "{stdout:?}");
    assert!(
        stdout.starts_with("entries ")
            && stdout.contains("\nindex bytes ")
            && stdout.ends_with('\n'),
        "{stdout:?}"
    );
    [found[0], found[1], found[2], found[3], found[4]]
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("list the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("a name")
        })
        .collect();
    names.sort();
    names
}

#[test]
fn every_remembered_entry_is_indexed_and_the_output_counts_them() {
    let sandbox = Sandbox::filled("indexed");
    let run = sandbox.reindex();
    assert_eq!(run.stderr, Vec::<String>::new());
    let [entries, chunks, unread, failed, bytes] = numbers(&run.stdout);
    assert_eq!((entries, unread, failed), (6, 0, 0));
    let (rows, hits) = sandbox.held();
    assert_eq!(rows.len() as u64, chunks);
    assert!(chunks >= entries, "{chunks}");
    let on_disk: u64 = names(&bilbo_tantivy::folder(&sandbox.cache()))
        .iter()
        .map(|name| {
            fs::metadata(bilbo_tantivy::folder(&sandbox.cache()).join(name))
                .expect("a file")
                .len()
        })
        .sum();
    assert_eq!(bytes, on_disk);
    let kinds: Vec<&str> = rows.iter().map(|row| row.kind.as_str()).collect();
    assert!(
        kinds.contains(&"session") && kinds.contains(&"markdown"),
        "{kinds:?}"
    );
    let entries_held: std::collections::BTreeSet<[u8; 16]> =
        rows.iter().map(|row| row.entry).collect();
    assert_eq!(entries_held.len(), 6);
    assert!(hits.iter().all(|found| !found.is_empty()), "{hits:?}");
    for row in &rows {
        assert!(
            row.first >= 1 && row.count >= 1 && row.generation == 1,
            "{row:?}"
        );
        assert!(row.date > 1_600_000_000_000, "{row:?}");
    }
}

#[test]
fn a_deleted_cache_folder_comes_back_the_same_and_so_does_a_second_run() {
    let sandbox = Sandbox::filled("same");
    let first = sandbox.reindex();
    let held = sandbox.held();
    let again = sandbox.reindex();
    assert_eq!(numbers(&again.stdout)[..4], numbers(&first.stdout)[..4]);
    assert_eq!(sandbox.held(), held, "a second run over the same store");
    fs::remove_dir_all(sandbox.dir.join("cache")).expect("delete the cache folder");
    let rebuilt = sandbox.reindex();
    assert_eq!(numbers(&rebuilt.stdout)[..4], numbers(&first.stdout)[..4]);
    assert_eq!(sandbox.held(), held, "after the cache folder was deleted");
    assert_eq!(names(&sandbox.cache()), ["catalog.sqlite", "index"]);
    fs::remove_dir_all(bilbo_tantivy::folder(&sandbox.cache())).expect("delete the index alone");
    sandbox.reindex();
    assert_eq!(sandbox.held(), held, "after the index alone was deleted");
}

#[test]
fn the_index_records_the_manifest_it_was_made_with_and_the_stores_cursor() {
    let sandbox = Sandbox::filled("manifest");
    let default = numbers(&sandbox.reindex().stdout);
    let lines: Vec<String> = sandbox.payload().lines().map(str::to_owned).collect();
    assert_eq!(
        lines[..6],
        [
            "bilbo index 2",
            "chunk_tokens 512",
            "chunk_overlap 64",
            "analyzer bm25s-en/2",
            "tokenizer qwen3-embedding-0.6b/97b0c614be4d/def76fb08697/raw",
            "render markdown/1/heading_width=100 session/1",
        ]
    );
    let store = sandbox.store();
    let (_, cursor) = store
        .changes(&bilbo_store::entries::Cursor::start())
        .expect("changes");
    let catalog: String = cursor
        .catalog()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        lines[6..],
        [
            format!("catalog {catalog}"),
            format!("seq {}", cursor.seq()),
            "failed -".to_owned(),
        ]
    );
    drop(store);

    let small = [
        ("BILBO_SEARCH_CHUNK_TOKENS", "256"),
        ("BILBO_SEARCH_CHUNK_OVERLAP", "16"),
    ];
    let run = sandbox.run_with(&["reindex"], &small, None);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        numbers(&run.stdout)[1] > default[1],
        "smaller chunks are more chunks"
    );
    assert!(
        sandbox
            .payload()
            .contains("\nchunk_tokens 256\nchunk_overlap 16\n")
    );

    let wide = sandbox.run_with(
        &["reindex"],
        &[("BILBO_MARKDOWN_HEADING_WIDTH", "40")],
        None,
    );
    assert_eq!(wide.code, 0, "{:?}", wide.stderr);
    assert!(
        sandbox
            .payload()
            .contains("\nrender markdown/1/heading_width=40 session/1\n")
    );
}

#[test]
fn only_the_newest_generation_of_an_entry_is_in_the_index() {
    let sandbox = Sandbox::new("generations");
    sandbox.write("notes/a.md", b"# One\n\nthe first text about zebras\n");
    sandbox.write("notes/b.md", b"# Other\n\nanother note\n");
    assert_eq!(sandbox.run(&["remember", &sandbox.path("notes")]).code, 0);
    sandbox.reindex();
    let before = sandbox.rank("zebras", 5);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].found.extent.generation, Generation(1));
    sandbox.write(
        "notes/a.md",
        b"# Rewritten\n\nnothing of the old text, only giraffes\n",
    );
    assert_eq!(sandbox.run(&["remember", &sandbox.path("notes")]).code, 0);
    sandbox.reindex();
    let index = sandbox.index();
    assert!(sandbox.rank("zebras", 5).is_empty());
    let after = sandbox.rank("giraffes", 5);
    assert_eq!(after.len(), 1);
    assert_eq!(
        (after[0].found.entry, after[0].found.extent.generation),
        (before[0].found.entry, Generation(2))
    );
    assert_eq!(index.documents().expect("documents").len(), 2);
}

#[test]
fn a_reindex_keeps_the_growth_stamps_so_the_next_remember_reads_no_unchanged_file() {
    let sandbox = Sandbox::filled("stamps");
    let stamps = |sandbox: &Sandbox| -> Vec<bool> {
        let store = sandbox.store();
        NOTES
            .iter()
            .map(|(name, _)| {
                let path = sandbox.dir.join("notes").join(name);
                let origin = Origin::file(&path).expect("an origin");
                store
                    .by_origin(&origin)
                    .expect("a head")
                    .expect("an entry")
                    .stamp
                    .is_some()
            })
            .collect()
    };
    assert_eq!(stamps(&sandbox), [true, true], "remember records them");
    sandbox.reindex();
    assert_eq!(stamps(&sandbox), [true, true], "reindex keeps them");
    fs::remove_dir_all(sandbox.dir.join("cache")).expect("delete the cache folder");
    sandbox.reindex();
    assert_eq!(
        stamps(&sandbox),
        [false, false],
        "a deleted catalog had the only copy"
    );
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!(
        run.stdout.lines().next(),
        Some("new 0, appended 0, new generation 0, unchanged 2, refused 0")
    );
    assert_eq!(
        stamps(&sandbox),
        [true, true],
        "the next remember records them again"
    );
}

#[test]
fn a_cache_folder_whose_index_folder_is_not_bilbos_is_refused_and_left_as_it_is() {
    let sandbox = Sandbox::filled("foreign");
    let folder = bilbo_tantivy::folder(&sandbox.cache());
    fs::remove_dir_all(&folder).expect("delete the index remember made");
    fs::create_dir_all(folder.join("models")).expect("a folder of someone else's");
    fs::write(folder.join("4887fa5108fbc055.vectors"), b"not bilbo's").expect("a file");
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: {} holds files that are not bilbo's index; bilbo will not touch it",
            folder.display()
        )]
    );
    assert_eq!(names(&folder), ["4887fa5108fbc055.vectors", "models"]);
    assert_eq!(
        fs::read(folder.join("4887fa5108fbc055.vectors")).expect("the file"),
        b"not bilbo's"
    );
}

#[test]
fn an_empty_store_reindexes_to_an_empty_index() {
    let sandbox = Sandbox::new("empty");
    let run = sandbox.reindex();
    assert_eq!(numbers(&run.stdout), [0, 0, 0, 0, numbers(&run.stdout)[4]]);
    assert_eq!(sandbox.index().documents().expect("documents"), []);
}

#[test]
fn a_reindex_waits_for_a_writer_of_the_store_and_says_so() {
    let sandbox = Sandbox::filled("waits");
    fs::remove_dir_all(bilbo_tantivy::folder(&sandbox.cache())).expect("delete the index");
    let state = sandbox.dir.join("state/bilbo");
    let lock = fs::File::open(state.join("store.lock")).expect("the lock file");
    lock.lock().expect("hold the store");
    let writer = fs::File::open(state.join("store.writer")).expect("the writer's lock file");
    writer.lock().expect("as a writer does");
    let (sender, receiver) = mpsc::channel();
    let vars = sandbox.vars();
    let runner = thread::spawn(move || bilbo(vars, &["reindex"], Some(sender)));
    assert_eq!(
        receiver.recv_timeout(WAIT).expect("a waiting notice"),
        "bilbo: waiting for another bilbo to finish writing the store"
    );
    assert!(
        !bilbo_tantivy::folder(&sandbox.cache()).exists(),
        "nothing is made while it waits"
    );
    writer.unlock().expect("release");
    lock.unlock().expect("release");
    let finished = runner.join().expect("the run finishes");
    assert_eq!(finished.code, 0, "{:?}", finished.stderr);
    assert_eq!(numbers(&finished.stdout)[0], 6);
}

struct Watched {
    inner: Box<dyn Index>,
    state: PathBuf,
    vars: HashMap<String, OsString>,
    bare: Arc<Mutex<Vec<(&'static str, Run)>>>,
}

impl Watched {
    fn meet(&self, point: &'static str) {
        for name in ["store.lock", "store.writer"] {
            let file = fs::File::open(self.state.join(name)).expect("a lock file");
            assert!(
                matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)),
                "{name} is held at {point}"
            );
        }
        let run = bilbo(self.vars.clone(), &["remember"], None);
        self.bare.lock().expect("runs").push((point, run));
    }
}

impl Index for Watched {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        self.inner.position()
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.meet("clear");
        self.inner.clear()
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        self.inner.add(doc)
    }

    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError> {
        self.meet("commit");
        self.inner.commit(position)
    }

    fn extents(&mut self, entry: Id) -> Result<Vec<Extent>, IndexError> {
        self.inner.extents(entry)
    }

    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError> {
        self.inner.delete(entry, from)
    }

    fn rollback(&mut self) {
        self.inner.rollback();
    }
}

#[test]
fn a_reindex_holds_the_store_from_the_catalog_to_the_commit_and_a_bare_remember_leaves_at_once() {
    let sandbox = Sandbox::filled("holds");
    fs::write(
        sandbox.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = {:?}\nkind = \"markdown\"\n",
            sandbox.path("notes")
        ),
    )
    .expect("a config that follows the notes");
    let bare = Arc::new(Mutex::new(Vec::new()));
    let watched = {
        let (state, vars, bare) = (
            sandbox.dir.join("state/bilbo"),
            sandbox.vars(),
            bare.clone(),
        );
        move |cache: &Path| -> Box<dyn Index> {
            Box::new(Watched {
                inner: disk(cache),
                state: state.clone(),
                vars: vars.clone(),
                bare: Arc::clone(&bare),
            })
        }
    };
    let mut env = Env {
        var: Box::new({
            let vars = sandbox.vars();
            move |name| vars.get(name).cloned()
        }),
        stdin: Box::new(io::empty()),
        notice: Box::new(|_| {}),
    };
    let settings = settings::load(&*env.var, &|path| fs::read(path), &bilbo_cli::kind_keys())
        .expect("settings");
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let rendering = bilbo_cli::rendering(&settings).expect("the rendering");
    let kinds = bilbo_cli::kinds(&settings).expect("the kinds");
    let indexing = bilbo_cli::indexing(
        &settings,
        &readers,
        &rendering,
        &bilbo_cli::tokenizer,
        &watched,
    );
    let outcome = bilbo_cli::reindex::run(
        &bilbo_cli::reindex::Args {},
        &mut env,
        &settings,
        &indexing,
        &|store, index, notice| {
            bilbo_cli::remember::follows::close(
                store,
                index,
                &settings.follow.value,
                &kinds,
                &indexing,
                notice,
                bilbo_cli::remember::follows::Progress::default(),
            )
        },
    )
    .expect("the reindex");
    assert_eq!(outcome.exit_code(), 0);
    let counts = numbers(&outcome.stdout(&View::PLAIN));
    assert_eq!((counts[0], counts[2], counts[3]), (6, 0, 0));
    let bare = bare.lock().expect("runs");
    let points: Vec<&str> = bare.iter().map(|(point, _)| *point).collect();
    assert_eq!(points, ["clear", "commit"]);
    for (point, run) in bare.iter() {
        assert_eq!(run.code, 0, "{point}: {:?}", run.stderr);
        assert_eq!(run.stdout, "", "{point}");
        assert_eq!(
            run.stderr,
            ["bilbo: another bilbo is writing the store; requested another pass over the follows"],
            "{point}"
        );
    }
    let said = outcome.stderr_lines(&View::PLAIN);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].starts_with("bilbo: ran the follows again for another bilbo: new 0, "),
        "{said:?}"
    );
    let flag = sandbox.dir.join("state/bilbo/store.again");
    assert_eq!(fs::metadata(&flag).expect("the flag").len(), 0);
}

#[test]
fn a_reindex_serves_the_again_flag_and_indexes_what_that_pass_kept() {
    let sandbox = Sandbox::filled("again");
    fs::write(
        sandbox.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = {:?}\nkind = \"markdown\"\n",
            sandbox.path("notes")
        ),
    )
    .expect("a config that follows the notes");
    sandbox.write("notes/new.md", b"# Fresh\n\na note about giraffes\n");
    let flag = sandbox.dir.join("state/bilbo/store.again");
    fs::write(&flag, b"1")
        .expect("ask for one more run, as a bare run that found the store held does");
    let run = sandbox.reindex();
    assert_eq!(
        numbers(&run.stdout)[0],
        6,
        "the rebuild counts what it found"
    );
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(
        run.stderr[0].starts_with("bilbo: ran the follows again for another bilbo: new 1, "),
        "{:?}",
        run.stderr
    );
    assert_eq!(fs::metadata(&flag).expect("the flag").len(), 0);
    let hits = sandbox.rank("giraffes", 5);
    assert_eq!(hits.len(), 1, "the pass's entry is in the index");
}

#[test]
fn a_remember_indexes_what_it_kept() {
    let sandbox = Sandbox::new("kept");
    sandbox.write("notes/a.md", b"# One\n\nthe first text about zebras\n");
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
    assert_eq!(sandbox.rank("zebras", 5).len(), 1);
    let first = sandbox.seq();
    sandbox.write("notes/b.md", b"# Two\n\nthe second text about giraffes\n");
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
    let second = sandbox.seq();
    assert!(second > first, "{first} then {second}");
    assert_eq!(sandbox.rank("zebras", 5).len(), 1);
    assert_eq!(sandbox.rank("giraffes", 5).len(), 1);
    assert!(
        sandbox.payload().ends_with("\nfailed -\n"),
        "{}",
        sandbox.payload()
    );
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
    assert_eq!(sandbox.seq(), second, "nothing new, nothing indexed");
}

/// A date is left out when it is the capture's own time: two stores are captured at different times.
type Named = Vec<(String, [u64; 5], Option<i64>, String, Vec<(String, u32)>)>;

fn named_terms(sandbox: &Sandbox, files: &[&str]) -> Named {
    let store = sandbox.store();
    let ids: Vec<([u8; 16], &str, i64)> = files
        .iter()
        .map(|name| {
            let origin = Origin::file(&sandbox.dir.join(name)).expect("an origin");
            let head = store.by_origin(&origin).expect("a head").expect("an entry");
            let meta = store
                .meta(head.id)
                .expect("a meta")
                .expect("an entry's meta");
            (head.id.to_bytes(), *name, millis(meta.captured))
        })
        .collect();
    let mut found: Named = sandbox
        .index()
        .terms()
        .expect("the terms")
        .into_iter()
        .map(|(row, words)| {
            let (_, name, captured) = ids
                .iter()
                .find(|(id, _, _)| *id == row.entry)
                .expect("a document of a known entry");
            (
                (*name).to_owned(),
                [row.generation, row.first, row.count, row.part, row.parts],
                (row.date != *captured).then_some(row.date),
                row.kind,
                words,
            )
        })
        .collect();
    found.sort();
    found
}

#[test]
fn a_session_remembered_turn_by_turn_holds_the_documents_of_a_reindex() {
    for (at, (name, bytes)) in SESSIONS.into_iter().enumerate() {
        let (other, other_bytes) = SESSIONS[(at + 1) % SESSIONS.len()];
        let path = format!("sessions/{name}");
        let other_path = format!("sessions/{other}");
        let grown = Sandbox::new("turns");
        grown.write(&other_path, other_bytes);
        let run = grown.run(&["remember", &grown.path(&other_path)]);
        assert_eq!(run.code, 0, "{other}: {:?}", run.stderr);
        let ends: Vec<usize> = bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(at, _)| at + 1)
            .collect();
        for end in ends {
            grown.write(&path, &bytes[..end]);
            let run = grown.run(&["remember", &grown.path(&path)]);
            assert_eq!(run.code, 0, "{name} cut at {end}: {:?}", run.stderr);

            let rebuilt = Sandbox::new("turns_reference");
            rebuilt.write(&other_path, other_bytes);
            rebuilt.write(&path, &bytes[..end]);
            for file in [&other_path, &path] {
                let run = rebuilt.run(&["remember", &rebuilt.path(file)]);
                assert_eq!(run.code, 0, "{file} cut at {end}: {:?}", run.stderr);
            }
            rebuilt.reindex();
            let files = [path.as_str(), other_path.as_str()];
            assert_eq!(
                named_terms(&grown, &files),
                named_terms(&rebuilt, &files),
                "{name} cut at {end}"
            );
        }
    }
}

fn open_fence_note(sections: usize) -> String {
    let mut note = String::from("---\n");
    for section in 0..sections.max(1) {
        if sections > 0 {
            note.push_str(&format!("# Section {section}\n\n"));
        }
        for line in 0..40 {
            note.push_str(&format!(
                "line {line} about the garden and its beans, carrots, peas, leeks and a very long list of other plants\n"
            ));
        }
    }
    note
}

#[test]
fn a_note_whose_frontmatter_closes_later_is_indexed_as_a_reindex_indexes_it() {
    let small = [("BILBO_SEARCH_CHUNK_TOKENS", "256")];
    for sections in [0, 3] {
        let sandbox = Sandbox::new("fence");
        let remember = |sandbox: &Sandbox| {
            let run = sandbox.run_with(&["remember", &sandbox.path("notes")], &small, None);
            assert_eq!(run.code, 0, "{:?}", run.stderr);
            run
        };
        let before = open_fence_note(sections);
        sandbox.write("notes/a.md", before.as_bytes());
        let first = remember(&sandbox);
        assert_eq!(
            first.stdout.lines().next(),
            Some("new 1, appended 0, new generation 0, unchanged 0, refused 0")
        );
        assert!(
            sandbox.index().documents().expect("documents").len() > 1,
            "more than one chunk"
        );
        let after = format!("{before}---\n\n# After the fence\n\nand a note about kiwis\n");
        sandbox.write("notes/a.md", after.as_bytes());
        let second = remember(&sandbox);
        assert_eq!(
            second.stdout.lines().next(),
            Some("new 0, appended 1, new generation 0, unchanged 0, refused 0"),
            "{sections} sections: the second capture is an append to the same generation"
        );
        let grown = sandbox.index().terms().expect("the terms");
        let run = sandbox.run_with(&["reindex"], &small, None);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        assert_eq!(
            sandbox.index().terms().expect("the terms"),
            grown,
            "{sections} sections"
        );
    }
}

#[test]
fn a_note_whose_last_paragraph_becomes_a_heading_is_indexed_as_a_reindex_indexes_it() {
    let small = [("BILBO_SEARCH_CHUNK_TOKENS", "256")];
    let sandbox = Sandbox::new("setext");
    let remember = |sandbox: &Sandbox| {
        let run = sandbox.run_with(&["remember", &sandbox.path("notes")], &small, None);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        run
    };
    let before = format!(
        "# Before\nshort text\n\n# Current\n\n{}",
        "garden beans carrots peas\n".repeat(400)
    );
    sandbox.write("notes/a.md", before.as_bytes());
    let first = remember(&sandbox);
    assert_eq!(
        first.stdout.lines().next(),
        Some("new 1, appended 0, new generation 0, unchanged 0, refused 0")
    );
    sandbox.write("notes/a.md", format!("{before}===\n").as_bytes());
    let second = remember(&sandbox);
    assert_eq!(
        second.stdout.lines().next(),
        Some("new 0, appended 1, new generation 0, unchanged 0, refused 0"),
        "the second capture is an append to the same generation"
    );
    let grown = sandbox.index().terms().expect("the terms");
    let grown_rows = sandbox.index().documents().expect("the documents");
    let run = sandbox.run_with(&["reindex"], &small, None);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        sandbox.index().documents().expect("the documents"),
        grown_rows
    );
    assert_eq!(sandbox.index().terms().expect("the terms"), grown);
}

#[test]
fn a_note_that_changes_is_indexed_again_whole() {
    let sandbox = Sandbox::new("changes");
    sandbox.write("notes/a.md", b"# One\n\nthe first text about zebras\n");
    sandbox.write("notes/b.md", b"# Other\n\nanother note\n");
    assert_eq!(sandbox.run(&["remember", &sandbox.path("notes")]).code, 0);
    sandbox.write(
        "notes/a.md",
        b"# One\n\nthe first text about zebras\n\n## More\n\nand some text about the garden\n",
    );
    assert_eq!(sandbox.run(&["remember", &sandbox.path("notes")]).code, 0);
    let grown = sandbox.matching();
    sandbox.reindex();
    assert_eq!(sandbox.matching(), grown, "a note that grew");
    sandbox.write(
        "notes/a.md",
        b"# Rewritten\n\nnothing of the old text, only giraffes\n",
    );
    assert_eq!(sandbox.run(&["remember", &sandbox.path("notes")]).code, 0);
    assert!(sandbox.rank("zebras", 5).is_empty());
    assert_eq!(sandbox.rank("giraffes", 5).len(), 1);
    let grown = sandbox.matching();
    sandbox.reindex();
    assert_eq!(sandbox.matching(), grown, "a note rewritten");
}

#[test]
fn a_remember_with_nothing_new_never_loads_the_tokenizer() {
    let sandbox = Sandbox::new("no_tokenizer");
    sandbox.write("notes/a.md", b"# One\n\nsome text\n");
    let notes = sandbox.path("notes");
    let loads = AtomicU64::new(0);
    let run = sandbox.remember_in_process(&notes, &disk, &loads);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(loads.load(Ordering::Relaxed), 1, "the first run has work");
    let run = sandbox.remember_in_process(&notes, &disk, &loads);
    assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
    assert_eq!(loads.load(Ordering::Relaxed), 1, "the second has none");
}

#[test]
fn a_changed_setting_rebuilds_on_the_next_remember_and_says_so() {
    let sandbox = Sandbox::filled("setting");
    let small = [("BILBO_SEARCH_CHUNK_TOKENS", "256")];
    let run = sandbox.run_with(&["remember", &sandbox.path("notes")], &small, None);
    assert_eq!(
        (run.code, run.stderr),
        (
            0,
            vec![
                "bilbo: rebuilding the search index: its settings or this bilbo's rules changed"
                    .to_owned()
            ]
        )
    );
    assert!(sandbox.payload().contains("\nchunk_tokens 256\n"));
}

#[test]
fn an_index_folder_that_is_not_bilbos_fails_the_run_after_the_capture_is_kept() {
    let sandbox = Sandbox::new("foreign_remember");
    let folder = bilbo_tantivy::folder(&sandbox.cache());
    fs::create_dir_all(folder.join("models")).expect("a folder of someone else's");
    sandbox.write("notes/a.md", b"# One\n\nsome text\n");
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout.lines().next(),
        Some("new 1, appended 0, new generation 0, unchanged 0, refused 0")
    );
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: the search index was not updated: {} holds files that are not bilbo's index; bilbo will not touch it",
            folder.display()
        )]
    );
    assert_eq!(names(&folder), ["models"]);
    let origin = Origin::file(&sandbox.dir.join("notes/a.md")).expect("an origin");
    assert!(
        sandbox
            .store()
            .by_origin(&origin)
            .expect("a head")
            .is_some()
    );
}

struct Flagger {
    inner: Box<dyn Index>,
    flag: PathBuf,
    ghost: Id,
    commits: usize,
    retried: Arc<AtomicU64>,
}

impl Index for Flagger {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        let mut found = self.inner.position()?;
        if let Some(found) = found.as_mut()
            && !found.failed.iter().any(|(id, _)| *id == self.ghost)
        {
            found.failed.push((self.ghost, 1));
        }
        Ok(found)
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.inner.clear()
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        self.inner.add(doc)
    }

    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError> {
        self.commits += 1;
        let stats = self.inner.commit(position)?;
        if self.commits == 2 {
            fs::write(&self.flag, b"1").expect("ask for one more run");
        }
        Ok(stats)
    }

    fn extents(&mut self, entry: Id) -> Result<Vec<Extent>, IndexError> {
        self.inner.extents(entry)
    }

    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError> {
        if entry == self.ghost {
            self.retried.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.delete(entry, from)
    }

    fn rollback(&mut self) {
        self.inner.rollback();
    }
}

#[test]
fn the_close_loop_indexes_what_each_pass_kept_and_retries_nothing() {
    let sandbox = Sandbox::filled("close_loop");
    fs::write(
        sandbox.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = {:?}\nkind = \"markdown\"\n",
            sandbox.path("notes")
        ),
    )
    .expect("a config that follows the notes");
    sandbox.write("notes/new.md", b"# Fresh\n\na note about giraffes\n");
    let flag = sandbox.dir.join("state/bilbo/store.again");
    fs::write(&flag, b"1").expect("the first request");
    let retried = Arc::new(AtomicU64::new(0));
    let flagger = {
        let (flag, retried) = (flag.clone(), Arc::clone(&retried));
        move |cache: &Path| -> Box<dyn Index> {
            Box::new(Flagger {
                inner: disk(cache),
                flag: flag.clone(),
                ghost: Id::from_bytes([9; 16]),
                commits: 0,
                retried: Arc::clone(&retried),
            })
        }
    };
    let run = sandbox.remember_in_process(&sandbox.path("sessions"), &flagger, &AtomicU64::new(0));
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr.len(), 2, "{:?}", run.stderr);
    assert!(
        run.stderr[0].starts_with("bilbo: ran the follows again for another bilbo: new 1, "),
        "{:?}",
        run.stderr
    );
    assert!(
        run.stderr[1].starts_with("bilbo: ran the follows again for another bilbo: new 0, "),
        "{:?}",
        run.stderr
    );
    assert_eq!(fs::metadata(&flag).expect("the flag").len(), 0);
    assert_eq!(
        retried.load(Ordering::Relaxed),
        1,
        "the first update tried the listed entry and no later pass did"
    );
    assert_eq!(sandbox.rank("giraffes", 5).len(), 1);
}

const BUILDING_SEVEN: &str = "bilbo: building the search index: 7 entries";

#[test]
fn a_first_remember_builds_the_index_and_says_nothing_of_it() {
    let sandbox = Sandbox::new("first");
    sandbox.write("notes/a.md", b"# One\n\nthe first text about zebras\n");
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
    assert_eq!(sandbox.index().documents().expect("documents").len(), 1);
}

#[test]
fn a_remember_that_finds_a_filled_store_with_no_index_says_how_many_entries_it_builds() {
    let sandbox = Sandbox::filled("no_index");
    fs::remove_dir_all(bilbo_tantivy::folder(&sandbox.cache())).expect("move the index away");
    sandbox.write("notes/new.md", b"# Fresh\n\na note about giraffes\n");
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!((run.code, run.stderr), (0, vec![BUILDING_SEVEN.to_owned()]));
    assert_eq!(sandbox.rank("giraffes", 5).len(), 1);
}

#[test]
fn the_line_that_an_index_is_being_built_is_written_before_the_tokenizer_is_loaded() {
    let sandbox = Sandbox::filled("line_first");
    fs::remove_dir_all(bilbo_tantivy::folder(&sandbox.cache())).expect("move the index away");
    sandbox.write("notes/new.md", b"# Fresh\n\na note about giraffes\n");
    let said = Arc::new(Mutex::new(Vec::new()));
    let seen = Mutex::new(None);
    let load = || {
        let mut seen = seen.lock().expect("seen");
        if seen.is_none() {
            *seen = Some(said.lock().expect("notices").clone());
        }
        bilbo_cli::tokenizer()
    };
    let run = sandbox.remember_with(&sandbox.path("notes"), &disk, &load, false, &said);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        seen.into_inner().expect("seen"),
        Some(vec![BUILDING_SEVEN.to_owned()]),
        "what the notice callback had been handed when the tokenizer was loaded"
    );
}

fn flag_at_two_captures(flag: PathBuf) -> bilbo_store::segments::Hook {
    let mut captures = 0;
    Box::new(move |step| {
        if step == bilbo_store::segments::Step::Appended && captures < 2 {
            captures += 1;
            fs::write(&flag, b"1")?;
        }
        Ok(())
    })
}

struct Refusing {
    inner: Box<dyn Index>,
    clears: Arc<AtomicU64>,
}

impl Index for Refusing {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        self.inner.position()
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.clears.fetch_add(1, Ordering::Relaxed);
        self.inner.clear()
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        self.inner.add(doc)
    }

    fn commit(&mut self, _: &Position) -> Result<Stats, IndexError> {
        self.inner.rollback();
        Err(IndexError::Failed {
            action: "commit",
            path: PathBuf::from("/refused"),
            error: "refused".to_owned(),
        })
    }

    fn extents(&mut self, entry: Id) -> Result<Vec<Extent>, IndexError> {
        self.inner.extents(entry)
    }

    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError> {
        self.inner.delete(entry, from)
    }

    fn rollback(&mut self) {
        self.inner.rollback();
    }
}

#[test]
fn an_update_that_fails_is_not_tried_again_by_the_close_loop() {
    let sandbox = Sandbox::filled("fatal");
    fs::write(
        sandbox.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = {:?}\nkind = \"markdown\"\n",
            sandbox.path("notes")
        ),
    )
    .expect("a config that follows the notes");
    fs::remove_dir_all(bilbo_tantivy::folder(&sandbox.cache())).expect("move the index away");
    sandbox.write("notes/first.md", b"# First\n\na note about zebras\n");
    sandbox.write("notes/second.md", b"# Second\n\na note about giraffes\n");
    let vars = sandbox.vars();
    let settings = settings::load(
        &|name| vars.get(name).cloned(),
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
    )
    .expect("settings");
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let rendering = bilbo_cli::rendering(&settings).expect("the rendering");
    let clears = Arc::new(AtomicU64::new(0));
    let refusing = {
        let clears = Arc::clone(&clears);
        move |cache: &Path| -> Box<dyn Index> {
            Box::new(Refusing {
                inner: disk(cache),
                clears: Arc::clone(&clears),
            })
        }
    };
    let indexing = bilbo_cli::indexing(
        &settings,
        &readers,
        &rendering,
        &bilbo_cli::tokenizer,
        &refusing,
    );
    let options = bilbo_store::segments::Options {
        hook: Some(flag_at_two_captures(
            sandbox.dir.join("state/bilbo/store.again"),
        )),
        ..settings.store.options()
    };
    let mut store = Store::open(&settings.layout(), options, &mut |_| {}).expect("open the store");
    let kinds = bilbo_cli::kinds(&settings).expect("the kinds");
    let first = bilbo_kinds::remember::plan(
        &bilbo_kinds::remember::Request {
            target: sandbox.dir.join("notes/first.md"),
            kind: None,
        },
        &kinds,
        &bilbo_cli::remember::disk::Disk,
    )
    .expect("a plan");
    bilbo_kinds::remember::keep(&first, &bilbo_cli::remember::disk::Disk, &mut store)
        .expect("the first capture");
    let notices = Mutex::new(Vec::new());
    let closing = bilbo_cli::remember::follows::finish(
        store,
        &settings.follow.value,
        &kinds,
        &indexing,
        &mut |message: &Message| {
            notices
                .lock()
                .expect("notices")
                .extend(message.plain_lines());
        },
        1,
    );
    assert_eq!(
        clears.load(Ordering::Relaxed),
        1,
        "the index was cleared once"
    );
    assert!(closing.refused && !closing.failed);
    let lines: Vec<String> = closing
        .messages
        .iter()
        .flat_map(Message::plain_lines)
        .collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(
        lines[0],
        "bilbo: the search index was not updated: cannot commit the index /refused: refused; bilbo reindex makes the index again"
    );
    assert!(
        lines[1].starts_with("bilbo: ran the follows again for another bilbo: new 1, "),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with("bilbo: ran the follows again for another bilbo: new 0, "),
        "{lines:?}"
    );
    assert_eq!(
        notices.into_inner().expect("notices"),
        ["bilbo: building the search index: 7 entries"]
    );
}

#[test]
fn an_entry_the_reader_fails_on_is_named_and_the_run_exits_1_with_the_capture_kept() {
    let sandbox = Sandbox::new("reader_fails");
    sandbox.write("notes/a.md", b"# One\n\nthe first text about zebras\n");
    sandbox.write("notes/b.md", b"# Two\n\nthe POISON text\n");
    let run = sandbox.remember_with(
        &sandbox.path("notes"),
        &disk,
        &bilbo_cli::tokenizer,
        true,
        &Arc::default(),
    );
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some("new 2, appended 0, new generation 0, unchanged 0, refused 0")
    );
    let store = sandbox.store();
    let origin = Origin::file(&sandbox.dir.join("notes/b.md")).expect("an origin");
    let head = store.by_origin(&origin).expect("a head").expect("kept");
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: left out {}: its reader or the tokenizer panicked",
            head.id
        )]
    );
    assert_eq!(sandbox.rank("zebras", 5).len(), 1);
    assert!(sandbox.rank("poison", 5).is_empty());
}

#[test]
fn a_reindex_seals_when_a_seal_is_due() {
    let sandbox = Sandbox::filled("seals");
    let sealed = |sandbox: &Sandbox| {
        fs::read_dir(sandbox.dir.join("store"))
            .expect("the store")
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_ok_and(|entry| entry.path().extension().is_some_and(|e| e == "seg"))
            })
            .count()
    };
    assert_eq!(sealed(&sandbox), 0);
    let hour_ago = SystemTime::now() - Duration::from_secs(61 * 60);
    let millis = hour_ago
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_millis();
    fs::write(
        sandbox.dir.join("state/bilbo/open.since"),
        format!("{millis}\n"),
    )
    .expect("backdate the open segment");
    sandbox.reindex();
    assert_eq!(sealed(&sandbox), 1, "the close of the reindex sealed it");
}

fn fnv(hash: u64, data: &[u8]) -> u64 {
    data.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "the done check is told its folders by the environment"
)]
fn folder(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name}")))
}

#[expect(clippy::disallowed_macros, reason = "the done check prints its counts")]
fn say(line: &str) {
    eprintln!("{line}");
}

/// `DONE_STORE`, `DONE_STATE` and `DONE_CACHE` are the three folders of a store `bilbo reindex` ran
/// over. Prints counts and digests only: two indexes of one store print the same lines.
#[test]
#[ignore = "the done check: an index built over a scratch store"]
fn what_an_index_of_the_done_check_store_holds() {
    const START: u64 = 0xcbf2_9ce4_8422_2325;
    let cache = folder("DONE_CACHE");
    let index = Tantivy::at(bilbo_tantivy::folder(&cache), options());
    let rows = index.documents().expect("the documents");
    let mut digest = START;
    for row in &rows {
        digest = fnv(digest, format!("{row:?}\n").as_bytes());
    }
    say(&format!(
        "index: {} documents, rows {digest:016x}",
        rows.len()
    ));
    let mut hits = START;
    let mut found = 0;
    for query in [
        "decide the store layout",
        "tantivy index commit",
        "why did the test fail",
        "garden",
        "reindex the catalog",
        "tokenizer",
        "error handling in the hook",
        "bazel build",
        "notes",
        "what did we decide about sync",
    ] {
        for candidate in rank(&cache, query, 50) {
            hits = fnv(
                hits,
                format!("{} {:?}\n", candidate.score.to_bits(), candidate.found).as_bytes(),
            );
            found += 1;
        }
    }
    say(&format!("index: {found} hits of 10 queries, {hits:016x}"));

    let layout = bilbo_store::layout::Layout {
        root: folder("DONE_STORE"),
        state: folder("DONE_STATE"),
        cache,
    };
    let store = Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("open");
    let settings = settings::load(
        &|name| (name == "HOME").then(|| "/nowhere".into()),
        &|_| Err(io::ErrorKind::NotFound.into()),
        &bilbo_cli::kind_keys(),
    )
    .expect("the defaults");
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let (changes, _) = store
        .changes(&bilbo_store::entries::Cursor::start())
        .expect("changes");
    let ids: std::collections::BTreeSet<_> = changes
        .iter()
        .map(bilbo_store::entries::Change::id)
        .collect();
    let (mut entries, mut passages, mut rendered, mut records) = (0_u64, 0_u64, 0_u64, 0_u64);
    for id in ids {
        let Some(meta) = store.meta(id).expect("a meta") else {
            continue;
        };
        let Some(reader) = readers.iter().find(|reader| reader.kind() == meta.kind) else {
            continue;
        };
        let (generation, count) = store
            .extent(id, None)
            .expect("an extent")
            .expect("an entry");
        if count == 0 {
            continue;
        }
        let range = bilbo_store::entries::RecordRange { first: 1, count };
        let kept = store.read(id, generation, range).expect("records");
        let kept: Vec<&[u8]> = kept.iter().map(Vec::as_slice).collect();
        let made = reader.passages(&kept);
        entries += 1;
        records += count;
        passages += made.len() as u64;
        rendered += made
            .iter()
            .map(|passage| passage.text.len() as u64 + 1)
            .sum::<u64>();
    }
    say(&format!(
        "store: {entries} entries, {records} records, {passages} passages, {rendered} bytes of rendered text"
    ));
}

#[test]
fn the_workers_key_changes_no_document() {
    let sandbox = Sandbox::filled("workers_key");
    let terms = |workers: &str| {
        let run = sandbox.run_with(&["reindex"], &[("BILBO_SEARCH_WORKERS", workers)], None);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        sandbox.index().terms().expect("the terms")
    };
    let alone = terms("1");
    assert!(!alone.is_empty());
    assert_eq!(terms("8"), alone);
}

#[test]
fn the_tries_key_reaches_the_update() {
    let second_run_of = |extra: &[(&str, &str)]| {
        let sandbox = Sandbox::new("tries_key");
        sandbox.write("notes/a.md", b"# One\n\nthe first text about zebras\n");
        sandbox.write("notes/b.md", b"# Two\n\nthe POISON text\n");
        let target = sandbox.path("notes");
        let kept = |sandbox: &Sandbox| {
            sandbox.remember_with_vars(
                &target,
                extra,
                &disk,
                &bilbo_cli::tokenizer,
                true,
                &Arc::default(),
            )
        };
        assert_eq!(kept(&sandbox).code, 1);
        kept(&sandbox)
    };
    let once = second_run_of(&[("BILBO_SEARCH_ENTRY_TRIES", "1")]);
    assert_eq!((once.code, once.stderr), (0, Vec::<String>::new()));
    let again = second_run_of(&[]);
    assert_eq!(again.code, 1, "{:?}", again.stderr);
    assert_eq!(again.stderr.len(), 1, "{:?}", again.stderr);
}
