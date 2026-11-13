use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::Env;
use bilbo_cli::console::indexing::Indexing;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings::{self, Settings};
use bilbo_cli::console::view::View;
use bilbo_cli::remember::disk::Disk;
use bilbo_kinds::read::refs::{Ref, Target};
use bilbo_kinds::remember::{self, Request};
use bilbo_kinds::tokens::Tokens;
use bilbo_search::analyze::{self, Detect, Language};
use bilbo_search::chunk;
use bilbo_search::index::IndexError;
use bilbo_search::lookup::{Filter, Found, Lookup, Posting, Totals};
use bilbo_search::position::Position;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_store::entries::{Entries, RecordRange};
use bilbo_store::segments::{Options, Store};
use rustix::fs::OFlags;

#[expect(
    dead_code,
    reason = "this target uses two helpers of the shared module"
)]
mod sandbox;

const SESSIONS: [(&str, &[u8]); 3] = [
    (
        "claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
        include_bytes!(
            "../../../libs/rust/kinds/session/fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
        ),
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
const FIRST_SESSION: &str = "sessions/claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl";

fn tokenizer() -> &'static Tokenizer {
    static LOADED: OnceLock<&'static Tokenizer> = OnceLock::new();
    LOADED.get_or_init(|| Box::leak(Box::new(bilbo_cli::tokenizer().expect("the tokenizer"))))
}

struct Shared;

impl Tokens for Shared {
    fn ends(&self, text: &str) -> Vec<usize> {
        tokenizer().tokens.ends(text)
    }

    fn count(&self, text: &str) -> usize {
        tokenizer().tokens.count(text)
    }

    fn prefix(&self, text: &str, budget: usize) -> usize {
        tokenizer().tokens.prefix(text, budget)
    }
}

fn shared_tokenizer() -> Result<Tokenizer, Failure> {
    Ok(Tokenizer {
        tokens: Box::new(Shared),
        id: tokenizer().id,
    })
}

fn tune(mut indexing: Indexing<'_>) -> Indexing<'_> {
    indexing.tokenizer = &shared_tokenizer;
    indexing
}

struct Run {
    code: u8,
    stdout: String,
    stderr: Vec<String>,
}

fn bilbo(vars: HashMap<String, OsString>, args: &[&str]) -> Run {
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let mut env = Env {
        var: Box::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        notice: Box::new(move |message| {
            sink.lock().expect("notices").extend(message.plain_lines());
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
        let dir = sandbox::scratch_dir(test);
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
        sandbox.remember(&["sessions", "notes"]);
        sandbox
    }

    fn remember(&self, folders: &[&str]) {
        for folder in folders {
            let run = self.run(&["remember", &self.path(folder)]);
            assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
        }
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_str().expect("UTF-8").to_owned()
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.dir.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(&path, bytes).expect("write");
        let file = fs::File::options().write(true).open(&path).expect("open");
        file.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .expect("backdate");
    }

    fn vars(&self) -> HashMap<String, OsString> {
        let mut vars = sandbox::vars(&self.dir);
        vars.insert("BILBO_SEARCH_WORKERS".to_owned(), OsString::from("2"));
        vars
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let mut vars = self.vars();
        vars.extend(
            extra
                .iter()
                .map(|(name, value)| ((*name).to_owned(), OsString::from(value))),
        );
        bilbo(vars, args)
    }

    fn settings(&self) -> Settings {
        settings::load(
            &|name| self.vars().get(name).cloned(),
            &|path| fs::read(path),
            &bilbo_cli::kind_keys(),
        )
        .expect("settings")
    }

    fn cache(&self) -> PathBuf {
        self.dir.join("cache/bilbo")
    }

    fn log(&self) -> PathBuf {
        self.dir.join("state/bilbo/queries.log")
    }

    fn tree(&self, except_log: bool) -> Vec<(PathBuf, u32, Option<Vec<u8>>)> {
        let mut found = Vec::new();
        for folder in ["store", "state", "cache"] {
            let mut stack = vec![self.dir.join(folder)];
            while let Some(dir) = stack.pop() {
                let mode =
                    |path: &PathBuf| fs::metadata(path).expect("metadata").permissions().mode();
                found.push((dir.clone(), mode(&dir), None));
                for entry in fs::read_dir(&dir).expect("list") {
                    let path = entry.expect("entry").path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if !(except_log && path == self.log()) {
                        let bytes = fs::read(&path).expect("read");
                        found.push((path.clone(), mode(&path), Some(bytes)));
                    }
                }
            }
        }
        found.sort();
        found
    }
}

struct Hit {
    reference: String,
    kind: String,
    day: String,
    extras: Vec<String>,
    content: String,
}

fn hits(stdout: &str) -> Vec<Hit> {
    let mut lines = stdout.split('\n').collect::<Vec<_>>();
    assert_eq!(lines.pop(), Some(""), "output ends with a line feed");
    let mut found = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let fields: Vec<&str> = lines[at]
            .strip_prefix("=== bilbo:")
            .unwrap_or_else(|| panic!("a header at {:?}", lines[at]))
            .split('\t')
            .collect();
        assert!(fields.len() >= 4, "{fields:?}");
        let count: usize = fields[3]
            .split(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .expect("a line count");
        let content = lines[at + 1..at + 1 + count].join("\n");
        found.push(Hit {
            reference: format!("bilbo:{}", fields[0]),
            kind: fields[1].to_owned(),
            day: fields[2].to_owned(),
            extras: fields[4..]
                .iter()
                .map(|field| (*field).to_owned())
                .collect(),
            content,
        });
        at += 1 + count;
    }
    found
}

#[test]
fn a_remembered_store_is_recalled_with_the_text_read_gives() {
    let sandbox = Sandbox::filled("recall_text");
    let run = sandbox.run(&["recall", "carrots", "beans", "garden"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stderr
            .iter()
            .any(|line| line.starts_with("bilbo: searched ") && line.ends_with(" entries")),
        "{:?}",
        run.stderr
    );
    let found = hits(&run.stdout);
    assert!(!found.is_empty(), "{}", run.stdout);

    let settings = sandbox.settings();
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let store = Store::open_read_only(&settings.layout(), Options::default(), &mut |_| {})
        .expect("open the store");
    let mut kinds = Vec::new();
    for hit in &found {
        let reference = Ref::parse(&hit.reference).expect("a ref bilbo read takes");
        let Target::Entry(id) = reference.target else {
            panic!("an entry id: {}", hit.reference);
        };
        let records = reference.locator.records.expect("records");
        let (generation, count) = store.extent(id, None).expect("extent").expect("an entry");
        let stored = store
            .read(id, generation, RecordRange { first: 1, count })
            .expect("records");
        let stored: Vec<&[u8]> = stored.iter().map(Vec::as_slice).collect();
        let reader = readers
            .iter()
            .find(|reader| reader.kind() == hit.kind)
            .expect("a reader of the kind");
        let part = hit.extras.iter().find_map(|extra| {
            let (at, of) = extra.strip_prefix("part ")?.split_once('/')?;
            Some((at.parse::<u32>().ok()? - 1, of.parse::<u32>().ok()?))
        });
        let chunk = chunk::at(
            &reader.passages(&stored),
            &Shared,
            settings.search.chunk(),
            records,
            part,
        )
        .expect("a chunk")
        .expect("the indexed chunk");
        let expected = match reader.preface(&stored) {
            Some(preface) => format!("{preface}\n{}", chunk.text),
            None => chunk.text,
        };
        assert_eq!(hit.content, expected, "{}", hit.reference);
        assert_eq!(hit.day.len(), 10, "{}", hit.day);
        kinds.push(hit.kind.as_str());
        let read = sandbox.run(&["read", &hit.reference]);
        assert_eq!(read.code, 0, "{:?}", read.stderr);
    }
    assert!(
        found
            .iter()
            .filter(|hit| hit.kind == "session")
            .all(|hit| hit.content.starts_with("Session started 20")
                || hit.content.starts_with("Session date unknown:")),
        "a session's hit opens with its kind's line"
    );
    assert!(kinds.contains(&"session"), "{kinds:?}");
}

#[test]
fn recall_writes_no_file_but_the_query_log() {
    let sandbox = Sandbox::filled("recall_writes");
    let before = sandbox.tree(false);
    assert!(!sandbox.log().exists());
    let queries: [(&[&str], u8, &str); 3] = [
        (&["recall", "carrots"], 0, "hits"),
        (&["recall", "heading"], 0, "hits"),
        (&["recall", "xyzzy"], 1, "nothing"),
    ];
    for (args, code, _) in queries {
        let run = sandbox.run(args);
        assert_eq!(run.code, code, "{args:?}: {:?}", run.stderr);
    }
    assert!(
        sandbox.tree(true) == before,
        "a recall changed the store, the state or the cache"
    );
    let log = fs::read_to_string(sandbox.log()).expect("the query log");
    let outcomes: Vec<&str> = log
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields[0], "q1", "{log}");
            fields[2]
        })
        .collect();
    assert_eq!(outcomes, queries.map(|(_, _, ended)| ended), "{log}");
    assert_eq!(
        fs::metadata(sandbox.log())
            .expect("the log")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn a_store_with_no_index_or_an_index_of_other_settings_names_reindex() {
    let sandbox = Sandbox::filled("recall_no_index");
    let reindex = "bilbo: the store has no search index bilbo can use; bilbo reindex makes it";
    fs::remove_dir_all(sandbox.cache()).expect("remove the cache");
    let run = sandbox.run(&["recall", "carrots"]);
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert_eq!(run.stderr, [reindex]);
    assert!(!sandbox.cache().exists(), "recall made the cache folder");

    let sandbox = Sandbox::filled("recall_other_settings");
    let run = sandbox.run_with(
        &["recall", "carrots"],
        &[("BILBO_SEARCH_CHUNK_TOKENS", "300")],
    );
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert_eq!(
        run.stderr,
        [
            "bilbo: the search index was made with other settings or by another bilbo; bilbo reindex makes it again"
        ]
    );
}

#[test]
fn a_session_that_grew_and_was_not_indexed_still_recalls_its_old_chunks_and_says_it_is_behind() {
    let sandbox = Sandbox::filled("recall_behind");
    let current = sandbox.run(&["recall", "carrots", "beans"]);
    assert_eq!(current.code, 0, "{:?}", current.stderr);
    assert!(
        !current.stderr.iter().any(|line| line.contains("behind")),
        "{:?}",
        current.stderr
    );

    let path = sandbox.dir.join(FIRST_SESSION);
    let mut grown = fs::read(&path).expect("the session");
    let last = grown[..grown.len() - 1]
        .split(|byte| *byte == b'\n')
        .rfind(|line| line.starts_with(b"{\"parentUuid\""))
        .expect("a record")
        .to_vec();
    grown.extend_from_slice(&last);
    grown.push(b'\n');
    fs::write(&path, &grown).expect("append to the session");
    let settings = sandbox.settings();
    let kinds = bilbo_cli::kinds(&settings).expect("the kinds");
    let plan = remember::plan(
        &Request {
            target: path,
            kind: None,
        },
        &kinds,
        &Disk,
    )
    .expect("a plan");
    let mut store =
        Store::open(&settings.layout(), Options::default(), &mut |_| {}).expect("open the store");
    remember::keep(&plan, &Disk, &mut store).expect("keep the growth");
    drop(store);

    let run = sandbox.run(&["recall", "carrots", "beans"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stdout, current.stdout, "the same chunks as before");
    assert!(
        run.stderr.contains(
            &"bilbo: the index is behind the store by 1 entry; bilbo remember brings it up to date"
                .to_owned()
        ),
        "{:?}",
        run.stderr
    );
}

#[test]
fn an_unknown_kind_is_a_usage_error_naming_the_kinds() {
    let sandbox = Sandbox::filled("recall_kind");
    let run = sandbox.run(&["recall", "--kind", "nope", "carrots"]);
    assert_eq!((run.code, run.stdout.as_str()), (2, ""));
    assert_eq!(
        run.stderr[0],
        "bilbo: unknown kind 'nope'; kinds: markdown, session"
    );
}

#[test]
fn a_budget_out_of_range_is_a_config_error_naming_the_flag() {
    let sandbox = Sandbox::filled("recall_budget");
    let run = sandbox.run(&["recall", "--budget", "10", "carrots"]);
    assert_eq!((run.code, run.stdout.as_str()), (2, ""));
    assert_eq!(
        run.stderr,
        ["bilbo: --budget must be a whole number from 256 to 1000000, got \"10\""]
    );
    let run = sandbox.run(&["recall", "--budget", "256", "carrots"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
}

#[test]
fn a_recall_waits_for_a_writer_as_read_does() {
    let sandbox = Sandbox::filled("recall_busy");
    let layout = sandbox.settings().layout();
    let hold = |name: &str| {
        let file = fs::File::options()
            .read(true)
            .write(true)
            .open(layout.state.join(name))
            .expect("a lock file");
        file.lock().expect("hold it as a writer does");
        file
    };
    let locks = [hold("store.lock"), hold("store.writer")];
    let at_once = sandbox.run_with(&["recall", "carrots"], &[("BILBO_STORE_WAIT", "0s")]);
    assert_eq!((at_once.code, at_once.stdout.as_str()), (1, ""));
    assert_eq!(
        at_once.stderr,
        ["bilbo: another bilbo is writing the store"]
    );
    let waited = sandbox.run_with(&["recall", "carrots"], &[("BILBO_STORE_WAIT", "1s")]);
    assert_eq!(waited.code, 1);
    assert_eq!(
        waited.stderr,
        [
            "bilbo: waiting for another bilbo to finish writing the store",
            "bilbo: another bilbo is writing the store"
        ]
    );
    drop(locks);
    let free = sandbox.run_with(&["recall", "carrots"], &[("BILBO_STORE_WAIT", "0s")]);
    assert_eq!(free.code, 0, "{:?}", free.stderr);
}

#[test]
fn a_cache_others_can_write_is_refused() {
    let sandbox = Sandbox::filled("recall_unsafe");
    fs::set_permissions(sandbox.cache(), fs::Permissions::from_mode(0o777)).expect("a mode");
    let run = sandbox.run(&["recall", "carrots"]);
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: {} is not this user's alone, or could not be checked; bilbo reads no search index under it",
            sandbox.cache().canonicalize().expect("canonical").display()
        )]
    );
}

#[test]
fn a_store_with_no_lock_file_is_not_searched_and_no_lock_is_made() {
    let sandbox = Sandbox::filled("recall_unlocked");
    let lock = sandbox.dir.join("state/bilbo/store.lock");
    fs::remove_file(&lock).expect("remove the lock file");
    let before = sandbox.tree(true);
    let run = sandbox.run(&["recall", "carrots"]);
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert_eq!(
        run.stderr,
        [
            "bilbo: no bilbo has written this store from this state folder, so nothing protects its search index while it is read; bilbo remember sets it up"
        ]
    );
    assert!(!lock.exists(), "recall made a lock file");
    assert!(sandbox.tree(true) == before, "recall changed a file");
}

#[test]
fn a_cache_of_another_store_is_not_searched_and_is_left_as_it_was() {
    let sandbox = Sandbox::filled("recall_other_store");
    let other = Sandbox::filled("recall_other_store_b");
    let foreign = other.cache();
    let before = other.tree(false);
    let mut vars = sandbox.vars();
    vars.insert(
        "XDG_CACHE_HOME".to_owned(),
        foreign.parent().expect("a parent").into(),
    );
    let run = bilbo(vars, &["recall", "carrots"]);
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: the cache folder {} holds another store's catalog; set paths.cache to a folder of this store's own",
            foreign.canonicalize().expect("canonical").display()
        )]
    );
    assert!(other.tree(false) == before, "the other store's files moved");
}

#[test]
fn the_question_words_key_changes_what_a_prompt_finds() {
    let sandbox = Sandbox::new("recall_question_words");
    sandbox.write(
        "notes/plans.md",
        b"# Plans\n\nanything goes in the garden shed\n",
    );
    let run = sandbox.run(&["remember", &sandbox.path("notes")]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let by_default = sandbox.run(&["recall", "anything"]);
    assert_eq!((by_default.code, by_default.stdout.as_str()), (1, ""));
    assert_eq!(
        by_default.stderr.last().map(String::as_str),
        Some("bilbo: nothing matched")
    );
    fs::write(
        sandbox.dir.join("config/config.toml"),
        "[recall]\nquestion_words = []\n",
    )
    .expect("a config file");
    let kept = sandbox.run(&["recall", "anything"]);
    assert_eq!(kept.code, 0, "{:?}", kept.stderr);
    assert!(
        kept.stdout.contains("anything goes in the garden shed"),
        "{}",
        kept.stdout
    );
}

fn state_folder(sandbox: &Sandbox) -> PathBuf {
    sandbox.dir.join("state/bilbo")
}

const REFUSED: &str = "is not a private file of this user in a folder of this user";
const NOT_LOGGED: &str = "bilbo: the query was not logged: ";

fn warning(run: &Run) -> Option<&String> {
    run.stderr.iter().find(|line| line.starts_with(NOT_LOGGED))
}

fn refuses(test: &str, ending: Option<&str>, sabotage: impl FnOnce(&Sandbox)) -> Sandbox {
    let sandbox = Sandbox::filled(test);
    let good = sandbox.run(&["recall", "carrots"]);
    assert_eq!(good.code, 0, "{:?}", good.stderr);
    assert!(warning(&good).is_none(), "{:?}", good.stderr);
    sabotage(&sandbox);
    let bad = sandbox.run(&["recall", "carrots"]);
    let said = warning(&bad).unwrap_or_else(|| panic!("no warning: {:?}", bad.stderr));
    if let Some(ending) = ending {
        assert!(said.ends_with(ending), "{said}");
    }
    assert_eq!((bad.code, &bad.stdout), (good.code, &good.stdout));
    let rest: Vec<&String> = bad.stderr.iter().filter(|line| *line != said).collect();
    assert_eq!(rest, good.stderr.iter().collect::<Vec<_>>());
    sandbox
}

fn mkfifo(path: &PathBuf) {
    let made = std::process::Command::new("mkfifo")
        .args(["-m", "600"])
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
}

fn read_end(path: &PathBuf) -> fs::File {
    use std::os::unix::fs::OpenOptionsExt;
    let flags = i32::try_from(OFlags::NONBLOCK.bits()).expect("flags");
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .expect("a reader")
}

#[test]
fn a_new_log_is_private_and_a_second_line_follows_the_first() {
    let sandbox = Sandbox::filled("recall_log_new");
    for _ in 0..2 {
        let run = sandbox.run(&["recall", "carrots"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        assert!(warning(&run).is_none(), "{:?}", run.stderr);
    }
    let log = sandbox.log();
    assert_eq!(
        fs::read_to_string(&log).expect("the log").lines().count(),
        2
    );
    assert_eq!(
        fs::metadata(&log).expect("the log").permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn a_log_that_is_a_link_is_not_written_through() {
    let mut outside = PathBuf::new();
    let sandbox = refuses("recall_log_link", None, |sandbox| {
        outside = sandbox.dir.join("outside.log");
        fs::write(&outside, "kept\n").expect("a target");
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).expect("a mode");
        fs::remove_file(sandbox.log()).expect("take the log away");
        symlink(&outside, sandbox.log()).expect("a link");
    });
    assert_eq!(fs::read_to_string(&outside).expect("the target"), "kept\n");
    drop(sandbox);

    let mut missing = PathBuf::new();
    let sandbox = refuses("recall_log_dangling", None, |sandbox| {
        missing = sandbox.dir.join("absent.log");
        fs::remove_file(sandbox.log()).expect("take the log away");
        symlink(&missing, sandbox.log()).expect("a dangling link");
    });
    assert!(!missing.exists(), "the link's target was created");
    drop(sandbox);
}

#[test]
fn a_log_others_can_read_is_not_written() {
    for bits in [0o644, 0o640, 0o660, 0o604] {
        let mut held = String::new();
        let sandbox = refuses("recall_log_mode", Some(REFUSED), |sandbox| {
            held = fs::read_to_string(sandbox.log()).expect("the log");
            fs::set_permissions(sandbox.log(), fs::Permissions::from_mode(bits)).expect("a mode");
        });
        let log = sandbox.log();
        assert_eq!(fs::read_to_string(&log).expect("the log"), held, "{bits:o}");
        assert_eq!(
            fs::metadata(&log).expect("the log").permissions().mode() & 0o777,
            bits
        );
    }
}

#[test]
fn a_log_with_a_second_name_is_not_written() {
    let mut held = String::new();
    let sandbox = refuses("recall_log_hard", Some(REFUSED), |sandbox| {
        held = fs::read_to_string(sandbox.log()).expect("the log");
        fs::hard_link(sandbox.log(), state_folder(sandbox).join("second")).expect("a second name");
    });
    assert_eq!(fs::read_to_string(sandbox.log()).expect("the log"), held);
}

#[test]
fn a_log_that_is_a_folder_is_not_written() {
    let sandbox = refuses("recall_log_folder", None, |sandbox| {
        fs::remove_file(sandbox.log()).expect("take the log away");
        fs::create_dir(sandbox.log()).expect("a folder");
    });
    assert_eq!(fs::read_dir(sandbox.log()).expect("the folder").count(), 0);
}

#[test]
fn a_log_that_is_a_fifo_with_no_reader_ends_the_run() {
    let sandbox = Sandbox::filled("recall_log_fifo_alone");
    let fifo = sandbox.log();
    mkfifo(&fifo);
    let vars = sandbox.vars();
    let (done, ended) = mpsc::channel();
    let runner = thread::spawn(move || {
        let run = bilbo(vars, &["recall", "carrots"]);
        done.send(run).expect("a result");
    });
    match ended.recv_timeout(Duration::from_secs(10)) {
        Ok(run) => {
            assert_eq!(run.code, 0, "{:?}", run.stderr);
            assert!(warning(&run).is_some(), "{:?}", run.stderr);
        }
        Err(_) => {
            let _free = read_end(&fifo);
            runner.join().expect("the thread");
            panic!("the run waited on a log nobody reads");
        }
    }
    runner.join().expect("the thread");
}

#[test]
fn a_log_that_is_a_fifo_with_a_reader_is_not_written() {
    let mut reader = None;
    refuses("recall_log_fifo", Some(REFUSED), |sandbox| {
        fs::remove_file(sandbox.log()).expect("take the log away");
        mkfifo(&sandbox.log());
        reader = Some(read_end(&sandbox.log()));
    });
    let mut bytes = Vec::new();
    let read = io::Read::read_to_end(&mut reader.expect("a reader"), &mut bytes);
    assert!(bytes.is_empty(), "{bytes:?} {read:?}");
}

#[test]
fn a_log_that_is_a_second_name_of_the_stores_lock_is_refused_and_the_run_ends() {
    let sandbox = Sandbox::filled("recall_log_lock_name");
    let good = sandbox.run(&["recall", "carrots"]);
    assert_eq!(good.code, 0, "{:?}", good.stderr);
    fs::remove_file(sandbox.log()).expect("take the log away");
    let lock = state_folder(&sandbox).join("store.lock");
    let held = fs::read(&lock).expect("the lock file");
    fs::hard_link(&lock, sandbox.log()).expect("a second name for the lock");
    let vars = sandbox.vars();
    let (done, ended) = mpsc::channel();
    thread::spawn(move || {
        done.send(bilbo(vars, &["recall", "carrots"])).ok();
    });
    let run = ended
        .recv_timeout(Duration::from_secs(10))
        .expect("the run ends");
    assert!(
        warning(&run).is_some_and(|said| said.ends_with(REFUSED)),
        "{:?}",
        run.stderr
    );
    assert_eq!((run.code, &run.stdout), (good.code, &good.stdout));
    assert_eq!(fs::read(&lock).expect("the lock file"), held);
    assert_eq!(fs::metadata(&lock).expect("the lock file").nlink(), 2);
}

#[test]
fn a_log_made_public_while_recall_waits_for_the_lock_is_not_written() {
    let sandbox = Sandbox::filled("recall_log_public_late");
    let good = sandbox.run(&["recall", "carrots"]);
    assert_eq!(good.code, 0, "{:?}", good.stderr);
    let held = fs::read_to_string(sandbox.log()).expect("the log");
    let lock = fs::File::options()
        .append(true)
        .open(sandbox.log())
        .expect("the log");
    lock.lock().expect("hold the log's lock");
    let vars = sandbox.vars();
    let (done, ended) = mpsc::channel();
    let runner = thread::spawn(move || {
        let run = bilbo(vars, &["recall", "carrots"]);
        done.send(run).expect("a result");
    });
    assert!(
        ended.recv_timeout(Duration::from_millis(1500)).is_err(),
        "the run ended while the log was locked"
    );
    fs::set_permissions(sandbox.log(), fs::Permissions::from_mode(0o644)).expect("a mode");
    lock.unlock().expect("let go");
    let run = ended
        .recv_timeout(Duration::from_secs(60))
        .expect("the run ends");
    runner.join().expect("the thread");
    assert!(
        warning(&run).is_some_and(|said| said.ends_with(REFUSED)),
        "{:?}",
        run.stderr
    );
    assert_eq!((run.code, &run.stdout), (good.code, &good.stdout));
    assert_eq!(fs::read_to_string(sandbox.log()).expect("the log"), held);
}

fn run_with_reading(
    sandbox: &Sandbox,
    loader: &dyn Fn() -> Result<Tokenizer, Failure>,
    lookup: &dyn Fn(&std::path::Path) -> Box<dyn bilbo_search::lookup::Lookup>,
) -> Result<bilbo_cli::console::outcome::Outcome, Failure> {
    run_with_detector(sandbox, loader, lookup, bilbo_cli::detect(), &["carrots"])
}

fn run_with_detector(
    sandbox: &Sandbox,
    loader: &dyn Fn() -> Result<Tokenizer, Failure>,
    lookup: &dyn Fn(&std::path::Path) -> Box<dyn bilbo_search::lookup::Lookup>,
    detect: &dyn bilbo_search::analyze::Detect,
    query: &[&str],
) -> Result<bilbo_cli::console::outcome::Outcome, Failure> {
    let settings = sandbox.settings();
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let render = bilbo_cli::rendering(&settings).expect("the rendering");
    let reading = bilbo_cli::console::reading::Reading {
        readers: &readers,
        chunk: settings.search.chunk(),
        render: &render,
        tokenizer: loader,
        lookup,
        detect,
    };
    let args = bilbo_cli::recall::Args {
        query: query.iter().map(|word| (*word).to_owned()).collect(),
        kind: Vec::new(),
        budget: None,
    };
    bilbo_cli::recall::run(
        &bilbo_cli::bilbo(),
        &args,
        &mut Env::quiet(),
        &settings,
        &reading,
    )
}

#[test]
fn a_folder_or_link_put_at_the_state_path_after_the_open_gets_no_log() {
    for linked in [false, true] {
        let sandbox = Sandbox::filled(if linked {
            "recall_log_swap_link"
        } else {
            "recall_log_swap_folder"
        });
        let state = state_folder(&sandbox);
        let aside = sandbox.dir.join("state/moved");
        let other = sandbox.dir.join("other-state");
        fs::create_dir(&other).expect("another folder");
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).expect("a mode");
        let lookup = |path: &std::path::Path| {
            fs::rename(&state, &aside).expect("move the state folder aside");
            if linked {
                symlink(&other, &state).expect("a link in its place");
            } else {
                fs::create_dir(&state).expect("a new folder");
                fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).expect("a mode");
            }
            bilbo_cli::lookup(path)
        };
        let outcome = run_with_reading(&sandbox, &shared_tokenizer, &lookup);
        let said = outcome
            .map(|outcome| outcome.stderr_lines(&View::PLAIN))
            .unwrap_or_default();
        assert!(
            !said.iter().any(|line| line.contains("not logged")),
            "{said:?}"
        );
        let log = fs::read_to_string(aside.join("queries.log")).expect("the moved folder's log");
        assert_eq!(log.lines().count(), 1, "{log}");
        assert!(
            !other.join("queries.log").exists(),
            "written through the link"
        );
        assert!(!state.join("queries.log").exists(), "written by the path");
    }
}

#[test]
fn a_state_folder_behind_a_users_link_is_still_logged() {
    let sandbox = Sandbox::new("recall_log_user_link");
    let real = sandbox.dir.join("state-real");
    fs::create_dir(&real).expect("a folder");
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).expect("a mode");
    fs::create_dir(sandbox.dir.join("state")).expect("the state home");
    symlink(&real, state_folder(&sandbox)).expect("the user's link");
    for (name, bytes) in NOTES {
        sandbox.write(&format!("notes/{name}"), bytes);
    }
    sandbox.remember(&["notes"]);
    let run = sandbox.run(&["recall", "carrots"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(warning(&run).is_none(), "{:?}", run.stderr);
    let log = fs::read_to_string(real.join("queries.log")).expect("the log");
    assert_eq!(log.lines().count(), 1, "{log}");
}

#[test]
fn a_store_with_no_state_folder_is_not_logged_and_says_so() {
    let sandbox = Sandbox::filled("recall_log_no_state");
    fs::remove_dir_all(state_folder(&sandbox)).expect("remove the state folder");
    let run = sandbox.run(&["recall", "carrots"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(
        run.stderr.contains(
            &"bilbo: the query was not logged: this store has no state folder that bilbo checked"
                .to_owned()
        ),
        "{:?}",
        run.stderr
    );
    assert!(
        !state_folder(&sandbox).exists(),
        "recall made the state folder"
    );
}

#[test]
fn an_append_waits_for_the_logs_lock() {
    let sandbox = Sandbox::filled("recall_log_lock");
    let run = sandbox.run(&["recall", "carrots"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let held = fs::File::options()
        .append(true)
        .open(sandbox.log())
        .expect("the log");
    held.lock().expect("hold the log's lock");
    let vars = sandbox.vars();
    let (done, ended) = mpsc::channel();
    let runner = thread::spawn(move || {
        let run = bilbo(vars, &["recall", "carrots"]);
        done.send(run.code).expect("a signal");
    });
    assert!(
        ended.recv_timeout(Duration::from_millis(1500)).is_err(),
        "the run ended while the log was locked"
    );
    held.unlock().expect("let go");
    assert_eq!(
        ended
            .recv_timeout(Duration::from_secs(60))
            .expect("the run ends"),
        0
    );
    runner.join().expect("the thread");
    assert_eq!(
        fs::read_to_string(sandbox.log())
            .expect("the log")
            .lines()
            .count(),
        2
    );
}

#[test]
fn the_manifest_names_the_tokenizer_that_counts() {
    let sandbox = Sandbox::filled("recall_manifest_id");
    let other = || {
        Ok(Tokenizer {
            tokens: Box::new(Shared),
            id: "another-tokenizer/1",
        })
    };
    let outcome = run_with_reading(&sandbox, &other, &bilbo_cli::lookup).expect("an outcome");
    assert_eq!(outcome.exit_code(), 1);
    assert_eq!(
        outcome.stderr_lines(&View::PLAIN),
        [
            "bilbo: the search index was made with other settings or by another bilbo; bilbo reindex makes it again"
        ]
    );
    let real =
        run_with_reading(&sandbox, &shared_tokenizer, &bilbo_cli::lookup).expect("an outcome");
    assert_eq!(real.exit_code(), 0, "{:?}", real.stderr_lines(&View::PLAIN));
}

#[test]
fn an_absent_cache_asks_for_no_lookup() {
    use std::cell::Cell;
    let sandbox = Sandbox::filled("recall_absent_cache");
    fs::remove_dir_all(sandbox.cache()).expect("remove the cache");
    let calls = Cell::new(0usize);
    let lookup = |path: &std::path::Path| {
        calls.set(calls.get() + 1);
        bilbo_cli::lookup(path)
    };
    let outcome = run_with_reading(&sandbox, &shared_tokenizer, &lookup).expect("an outcome");
    assert_eq!(outcome.exit_code(), 1);
    assert_eq!(
        outcome.stderr_lines(&View::PLAIN),
        ["bilbo: the store has no search index bilbo can use; bilbo reindex makes it"]
    );
    assert_eq!(calls.get(), 0);
    assert!(!sandbox.cache().exists(), "recall made the cache folder");
    let log = fs::read_to_string(sandbox.log()).expect("the log");
    assert_eq!(
        log.lines().last().and_then(|line| line.split('\t').nth(2)),
        Some("no-index")
    );
}

#[test]
fn the_read_batch_key_bounds_each_read_of_the_index() {
    let sandbox = Sandbox::filled("recall_read_batch");
    let widest_read = |config: &str| {
        fs::write(sandbox.dir.join("config/config.toml"), config).expect("a config file");
        let seen = Arc::new(Seen::default());
        let lookup = recording(&seen);
        let outcome = run_with_reading(&sandbox, &shared_tokenizer, &lookup).expect("an outcome");
        assert_eq!(outcome.exit_code(), 0);
        seen.widest.load(Ordering::Relaxed)
    };
    assert!(widest_read("") > 1);
    assert_eq!(widest_read("[recall]\nread_batch = 1\n"), 1);
}

fn long_note(word: &str) -> String {
    let lines: Vec<String> = (1..=200)
        .map(|n| format!("line {n} about {word} grown in the east bed with some more words"))
        .collect();
    format!("# Long\n\n{}\n", lines.join("\n"))
}

#[test]
fn a_kind_narrows_the_hits_of_a_query_both_kinds_match() {
    let sandbox = Sandbox::filled("recall_kinds");
    let all = sandbox.run(&["recall", "carrots"]);
    assert_eq!(all.code, 0, "{:?}", all.stderr);
    let found = hits(&all.stdout);
    for kind in ["markdown", "session"] {
        assert!(
            found.iter().any(|hit| hit.kind == kind),
            "no {kind} hit without --kind"
        );
        let narrowed = sandbox.run(&["recall", "--kind", kind, "carrots"]);
        assert_eq!(narrowed.code, 0, "{:?}", narrowed.stderr);
        let only = hits(&narrowed.stdout);
        assert!(!only.is_empty());
        assert!(
            only.iter().all(|hit| hit.kind == kind),
            "{}",
            narrowed.stdout
        );
        assert!(
            only.len() < found.len(),
            "{} against {}",
            only.len(),
            found.len()
        );
    }
}

#[test]
fn a_lower_budget_cuts_a_hit_and_prints_less() {
    let sandbox = Sandbox::new("recall_budget_cut");
    sandbox.write("notes/long.md", long_note("turnips").as_bytes());
    sandbox.remember(&["notes"]);
    let full = sandbox.run(&["recall", "turnips"]);
    let small = sandbox.run(&["recall", "--budget", "256", "turnips"]);
    assert_eq!((full.code, small.code), (0, 0), "{:?}", small.stderr);
    let (full_hits, small_hits) = (hits(&full.stdout), hits(&small.stdout));
    let cut = small_hits.last().expect("a hit");
    assert_eq!(cut.extras.last().map(String::as_str), Some("cut"));
    let same = full_hits
        .iter()
        .find(|hit| hit.reference == cut.reference)
        .expect("the same hit");
    assert!(cut.content.lines().count() < same.content.lines().count());
    assert!(small.stdout.len() < full.stdout.len());
}

#[test]
fn the_candidates_key_bounds_the_hits() {
    let sandbox = Sandbox::new("recall_candidates");
    sandbox.write("notes/long.md", long_note("turnips").as_bytes());
    sandbox.remember(&["notes"]);
    let all = sandbox.run(&["recall", "turnips"]);
    assert!(hits(&all.stdout).len() >= 3, "{}", all.stdout);
    let one = sandbox.run_with(&["recall", "turnips"], &[("BILBO_RECALL_CANDIDATES", "1")]);
    assert_eq!(hits(&one.stdout).len(), 1, "{}", one.stdout);
}

#[test]
fn the_cap_key_bounds_the_hits_of_one_entry() {
    let sandbox = Sandbox::new("recall_cap");
    sandbox.write("notes/long.md", long_note("turnips").as_bytes());
    sandbox.remember(&["notes"]);
    let all = sandbox.run(&["recall", "turnips"]);
    assert!(hits(&all.stdout).len() >= 2, "{}", all.stdout);
    let capped = sandbox.run_with(&["recall", "turnips"], &[("BILBO_RECALL_CAP", "1")]);
    assert_eq!(hits(&capped.stdout).len(), 1, "{}", capped.stdout);
}

fn first_marker(run: &Run, markers: &[&str]) -> String {
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let first = hits(&run.stdout).into_iter().next().expect("a hit");
    markers
        .iter()
        .find(|marker| first.content.contains(**marker))
        .unwrap_or_else(|| panic!("no marker in {}", first.content))
        .to_string()
}

#[test]
fn k1_and_b_each_change_which_note_comes_first() {
    let filler = vec!["filler"; 297].join(" ");
    let sandbox = Sandbox::new("recall_b");
    sandbox.write("notes/a.md", b"# A\n\nshortnote zorblat alpha beta\n");
    sandbox.write(
        "notes/b.md",
        format!("# B\n\nlongnote {filler} zorblat zorblat zorblat\n").as_bytes(),
    );
    sandbox.remember(&["notes"]);
    let markers = ["shortnote", "longnote"];
    let b0 = sandbox.run_with(&["recall", "zorblat"], &[("BILBO_RECALL_BM25_B", "0")]);
    assert_eq!(first_marker(&b0, &markers), "longnote");
    let b1 = sandbox.run_with(&["recall", "zorblat"], &[("BILBO_RECALL_BM25_B", "1")]);
    assert_eq!(first_marker(&b1, &markers), "shortnote");

    let sandbox = Sandbox::new("recall_k1");
    sandbox.write("notes/c.md", b"# C\n\ncnote zorblat quimby\n");
    sandbox.write(
        "notes/d.md",
        b"# D\n\ndnote zorblat zorblat zorblat zorblat zorblat zorblat zorblat zorblat zorblat zorblat\n",
    );
    sandbox.remember(&["notes"]);
    let markers = ["cnote", "dnote"];
    let low = |k1: &'static str| {
        sandbox.run_with(
            &["recall", "zorblat", "quimby"],
            &[("BILBO_RECALL_BM25_B", "0"), ("BILBO_RECALL_BM25_K1", k1)],
        )
    };
    assert_eq!(first_marker(&low("0.01"), &markers), "cnote");
    assert_eq!(first_marker(&low("10"), &markers), "dnote");
}

#[test]
#[ignore = "the done check: EVAL_CORPUS and EVAL_STORE name the unpacked retrieval eval's corpus and bm25 store"]
fn the_eval_corpus_remembered_gives_the_eval_stores_chunks() {
    let (corpus, eval) = (folder("EVAL_CORPUS"), folder("EVAL_STORE"));
    let sandbox = Sandbox::new("eval_capture");
    let settings = sandbox.settings();
    let kinds = bilbo_cli::kinds(&settings).expect("the kinds");
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let reader = readers
        .iter()
        .find(|reader| reader.kind() == "session")
        .expect("the session reader");
    let mut store =
        Store::open(&settings.layout(), Options::default(), &mut |_| {}).expect("open the store");
    let mut repos: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("the corpus")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.is_dir())
        .collect();
    repos.sort();
    for repo in &repos {
        let plan = remember::plan(
            &Request {
                target: repo.clone(),
                kind: Some("session".to_owned()),
            },
            &kinds,
            &Disk,
        )
        .expect("a plan");
        remember::keep(&plan, &Disk, &mut store).expect("keep the repo");
    }

    let mut expected: std::collections::BTreeMap<(String, String), Vec<serde_json::Value>> =
        std::collections::BTreeMap::new();
    let chunks = fs::read_to_string(eval.join("chunks.jsonl")).expect("chunks.jsonl");
    for line in chunks.lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("a row");
        let key = (
            row["repo"].as_str().expect("a repo").to_owned(),
            row["session"].as_str().expect("a session").to_owned(),
        );
        expected.entry(key).or_default().push(row);
    }
    let (mut all, mut missing, mut differ) = (0u64, 0u64, 0u64);
    for ((repo, session), rows) in expected {
        let path = corpus.join(&repo).join(&session);
        let head = bilbo_store::entries::Origin::file(&path)
            .and_then(|origin| store.by_origin(&origin).expect("a lookup"));
        let made: std::collections::HashMap<_, _> = head
            .map(|head| {
                let stored = store
                    .read(
                        head.id,
                        head.generation,
                        RecordRange {
                            first: 1,
                            count: head.records,
                        },
                    )
                    .expect("records");
                let stored: Vec<&[u8]> = stored.iter().map(Vec::as_slice).collect();
                chunk::chunks(&reader.passages(&stored), &Shared, settings.search.chunk())
                    .expect("chunks")
                    .into_iter()
                    .map(|chunk| {
                        let last = chunk.records.first + chunk.records.count - 1;
                        ((chunk.records.first, last, chunk.part), chunk.text)
                    })
                    .collect()
            })
            .unwrap_or_default();
        for row in rows {
            all += 1;
            let line = |at: usize| row["lines"][at].as_u64().expect("a line");
            let part = row["part"].as_array().map(|part| {
                let number =
                    |at: usize| u32::try_from(part[at].as_u64().expect("a part")).expect("a part");
                (number(0), number(1))
            });
            let same = match made.get(&(line(0), line(1), part)) {
                None => {
                    missing += 1;
                    false
                }
                Some(text) if Some(text.as_str()) == row["text"].as_str() => true,
                Some(_) => {
                    differ += 1;
                    false
                }
            };
            if !same {
                let part = part.map_or_else(|| "-".to_owned(), |(at, of)| format!("{at}/{of}"));
                say(&format!(
                    "{repo}\t{session}\t{}\t{}\t{part}",
                    line(0),
                    line(1)
                ));
            }
        }
    }
    say(&format!(
        "eval chunks {all}: {missing} with no chunk at their coordinates, {differ} with other text"
    ));
    assert!(all > 100_000, "{all}");
    assert_eq!((missing, differ), (0, 0));
}

#[expect(
    clippy::disallowed_methods,
    reason = "a done check is told its folders by the environment"
)]
fn folder(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} names a folder")))
}

#[expect(clippy::disallowed_macros, reason = "the done check prints its counts")]
fn say(line: &str) {
    eprintln!("{line}");
}

struct Portuguese(AtomicUsize);

impl Detect for Portuguese {
    fn language(&self, _: &str) -> Option<Language> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Some(Language::Portuguese)
    }
}

#[derive(Default)]
struct Seen {
    terms: Mutex<BTreeSet<String>>,
    widest: AtomicUsize,
}

struct Recording {
    inner: Box<dyn Lookup>,
    seen: Arc<Seen>,
}

impl Lookup for Recording {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.inner.position()
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.inner.totals()
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        self.seen
            .terms
            .lock()
            .expect("terms")
            .insert(term.to_owned());
        self.inner.postings(term)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.inner.holding(filter)
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.seen.widest.fetch_max(docs.len(), Ordering::Relaxed);
        self.inner.found(docs)
    }
}

fn recording(seen: &Arc<Seen>) -> impl Fn(&std::path::Path) -> Box<dyn Lookup> {
    let seen = Arc::clone(seen);
    move |path| {
        Box::new(Recording {
            inner: bilbo_cli::lookup(path),
            seen: Arc::clone(&seen),
        })
    }
}

#[test]
fn the_languages_key_reaches_the_search() {
    let sandbox = Sandbox::filled("recall_languages_key");
    let ask = || {
        let detector = Portuguese(AtomicUsize::new(0));
        let seen = Arc::new(Seen::default());
        run_with_detector(
            &sandbox,
            &shared_tokenizer,
            &recording(&seen),
            &detector,
            &["que", "pelo", "carrots"],
        )
        .expect("the run ends");
        let terms = seen.terms.lock().expect("terms").clone();
        (detector.0.load(Ordering::Relaxed), terms)
    };
    let terms = |text: &str| -> BTreeSet<String> { analyze::terms(text).into_iter().collect() };
    assert_eq!(
        ask(),
        (1, terms("carrots")),
        "the listed words are not looked up"
    );
    fs::write(
        sandbox.dir.join("config/config.toml"),
        "[recall]\nlanguages = []\n",
    )
    .expect("a config file");
    assert_eq!(
        ask(),
        (0, terms("carrots pelo que")),
        "the listed words are looked up"
    );
}

#[test]
fn the_cli_hands_recall_whichlang() {
    let detect = bilbo_cli::detect();
    assert_eq!(
        detect.language("Olá feliz contribuinte"),
        Some(Language::Portuguese)
    );
    assert_eq!(detect.language("Hello, happy tax payer"), None);
}
