use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::Env;
use bilbo_cli::console::indexing::Indexing;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings::{self, Settings};
use bilbo_cli::console::view::View;
use bilbo_cli::remember::disk::Disk;
use bilbo_entries::read::refs::{Ref, Target};
use bilbo_entries::remember::{self, Request};
use bilbo_entries::tokens::Tokens;
use bilbo_search::analyze::{self, Detect, Language};
use bilbo_search::chunk;
use bilbo_search::index::IndexError;
use bilbo_search::lookup::{Edits, Filter, Found, Lookup, Near, Posting, Totals};
use bilbo_search::position::Position;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_store::entries::{Entries, Meta, RecordRange};
use bilbo_store::segments::{Options, Store};
use rustix::fs::OFlags;

mod note_line;
mod notes;
mod runner;
mod scratch;

use runner::{Run, Sandbox, Shared};

const SESSIONS: [(&str, &[u8]); 3] = [
    (
        "claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
        include_bytes!(
            "../../../libs/entries/rust/fixtures/session/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
        ),
    ),
    (
        "claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl",
        include_bytes!(
            "../../../libs/entries/rust/fixtures/session/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
        ),
    ),
    (
        "codex/2026/11/07/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl",
        include_bytes!(
            "../../../libs/entries/rust/fixtures/session/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
        ),
    ),
];
const NOTES: [(&str, &[u8]); 2] = [
    (
        "sections.md",
        include_bytes!("../../../libs/entries/rust/fixtures/markdown/sections.md"),
    ),
    (
        "bare.md",
        include_bytes!("../../../libs/entries/rust/fixtures/markdown/no-frontmatter.md"),
    ),
];
const FIRST_SESSION: &str = "sessions/claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl";

fn tune(mut indexing: Indexing<'_>) -> Indexing<'_> {
    indexing.tokenizer = &runner::shared_tokenizer;
    indexing
}

fn bilbo(vars: HashMap<String, OsString>, args: &[&str]) -> Run {
    runner::bilbo_with(
        std::rc::Rc::new(move |name| vars.get(name).cloned()),
        args,
        &tune,
    )
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = scratch::scratch_dir(test);
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
        let mut vars = scratch::vars(&self.dir);
        vars.insert("BILBO_SEARCH_WORKERS".to_owned(), OsString::from("2"));
        vars
    }

    fn vars_with(&self, extra: &[(&str, &str)]) -> HashMap<String, OsString> {
        let mut vars = self.vars();
        vars.extend(
            extra
                .iter()
                .map(|(name, value)| ((*name).to_owned(), OsString::from(value))),
        );
        vars
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let vars = self.vars_with(extra);
        bilbo(vars, args)
    }

    fn settings(&self) -> Settings {
        self.settings_with(&[])
    }

    fn settings_with(&self, extra: &[(&str, &str)]) -> Settings {
        let vars = self.vars_with(extra);
        settings::load(
            &|name| vars.get(name).cloned(),
            &|path| fs::read(path),
            &bilbo_cli::kind_keys(),
            &bilbo_cli::provider_keys(),
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
    extras: Vec<String>,
    content: String,
    printed: String,
}

fn entry_of(store: &Store, reference: &Ref) -> bilbo_store::id::Id {
    match &reference.target {
        Target::Entry(id) => *id,
        Target::Short(short) => {
            let ids = store.by_shorts(&[*short]).expect("a lookup");
            assert_eq!(ids.len(), 1, "{short} ends one entry's id");
            ids[0]
        }
        Target::Session(session) => panic!("a header prints no session id: {session}"),
    }
}

fn entries_named(sandbox: &Sandbox, run: &Run) -> Vec<bilbo_store::id::Id> {
    let store = Store::open_read_only(
        &sandbox.settings().layout(),
        Options::default(),
        &mut |_| {},
    )
    .expect("open the store");
    hits(&run.stdout)
        .iter()
        .map(|hit| entry_of(&store, &Ref::parse(&hit.reference).expect("a ref")))
        .collect()
}

fn meta_of(sandbox: &Sandbox, hit: &Hit) -> Meta {
    let settings = sandbox.settings();
    let store = Store::open_read_only(&settings.layout(), Options::default(), &mut |_| {})
        .expect("open the store");
    let reference = Ref::parse(hit.reference.as_str()).expect("a ref bilbo read takes");
    let id = entry_of(&store, &reference);
    store.meta(id).expect("meta").expect("an entry")
}

fn capture_of(sandbox: &Sandbox, hit: &Hit) -> String {
    meta_of(sandbox, hit)
        .labels
        .capture
        .expect("a capture label")
        .to_string()
}

fn hits(stdout: &str) -> Vec<Hit> {
    let mut lines = stdout.split('\n').collect::<Vec<_>>();
    assert_eq!(lines.pop(), Some(""), "output ends with a line feed");
    let mut found = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let fields: Vec<&str> = lines[at]
            .strip_prefix("=== ")
            .unwrap_or_else(|| panic!("a header at {:?}", lines[at]))
            .split('\t')
            .collect();
        assert!(fields.len() >= 2, "{fields:?}");
        let id = fields[0].split('#').next().expect("an id");
        assert!(matches!(id.len(), 10 | 26), "{fields:?}");
        let count: usize = fields[1]
            .split(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .expect("a line count");
        let content = lines[at + 1..at + 1 + count].join("\n");
        let printed = format!("{}\n{content}\n", lines[at]);
        found.push(Hit {
            reference: fields[0].to_owned(),
            extras: fields[2..]
                .iter()
                .map(|field| (*field).to_owned())
                .collect(),
            content,
            printed,
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
        let id = entry_of(&store, &reference);
        let kind = store.meta(id).expect("meta").expect("an entry").kind;
        let records = reference.locator.records.expect("records");
        let (generation, count) = store.extent(id, None).expect("extent").expect("an entry");
        let stored = store
            .read(id, generation, RecordRange { first: 1, count })
            .expect("records");
        let stored: Vec<&[u8]> = stored.iter().map(Vec::as_slice).collect();
        let reader = readers
            .iter()
            .find(|reader| reader.kind() == kind)
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
        kinds.push(kind);
        let read = sandbox.run(&["read", &hit.reference]);
        assert_eq!(read.code, 0, "{:?}", read.stderr);
    }
    for (hit, kind) in found.iter().zip(&kinds) {
        assert!(
            kind != "session"
                || hit.content.starts_with("Session started 20")
                || hit.content.starts_with("Session date unknown:"),
            "a session's hit opens with its kind's line"
        );
    }
    assert!(kinds.iter().any(|kind| kind == "session"), "{kinds:?}");
}

#[test]
fn recall_writes_no_file_but_the_query_log() {
    let sandbox = Sandbox::filled("recall_writes");
    let before = sandbox.tree(false);
    assert!(!sandbox.log().exists());
    let queries: [(&[&str], u8, &str); 4] = [
        (&["recall", "carrots"], 0, "hits"),
        (&["recall", "carots"], 0, "hits"),
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
            assert_eq!((fields[0], fields.len()), ("q3", 10), "{log}");
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

fn typo_notes(test: &str) -> Sandbox {
    let sandbox = Sandbox::new(test);
    sandbox.write(
        "notes/garden.md",
        b"# Garden\n\nPlanted carrots in the north bed.\n",
    );
    sandbox.write(
        "notes/aviary.md",
        b"# Aviary\n\nThe parrots sit by the window.\n",
    );
    sandbox.remember(&["notes"]);
    sandbox
}

#[test]
fn a_misspelt_query_finds_its_note_and_says_so() {
    let sandbox = typo_notes("recall_typo_note");
    let run = sandbox.run(&["recall", "carots"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let found = hits(&run.stdout);
    assert_eq!(found.len(), 1, "{}", run.stdout);
    assert!(found[0].content.contains("carrots"), "{}", run.stdout);
    assert!(
        run.stderr
            .contains(&"bilbo: typos: carot as carrot".to_owned()),
        "{:?}",
        run.stderr
    );

    let right = sandbox.run(&["recall", "carrots"]);
    assert_eq!(right.code, 0, "{:?}", right.stderr);
    let found = hits(&right.stdout);
    assert_eq!(found.len(), 1, "{}", right.stdout);
    assert!(found[0].content.contains("carrots"), "{}", right.stdout);
    assert!(
        right.stderr.iter().all(|line| !line.contains("typos:")),
        "{:?}",
        right.stderr
    );

    let log = fs::read_to_string(sandbox.log()).expect("the query log");
    let typos: Vec<(String, String)> = log
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!((fields[0], fields.len()), ("q3", 10), "{log}");
            (fields[8].to_owned(), fields[9].to_owned())
        })
        .collect();
    assert_eq!(
        typos,
        [
            ("carot>carrot".to_owned(), "carots".to_owned()),
            ("-".to_owned(), "carrots".to_owned())
        ]
    );
}

#[test]
fn the_typo_keys_reach_the_search() {
    let sandbox = typo_notes("recall_typo_keys");
    let ask = |query: &str, extra: &[(&str, &str)]| {
        let seen = Arc::new(Seen::default());
        let done = run_with_settings(
            &sandbox.settings_with(extra),
            &runner::shared_tokenizer,
            &recording(&seen),
            bilbo_cli::detect(),
            &[query],
        )
        .expect("the run ends");
        let near = seen.near.lock().expect("near").clone();
        (done.exit_code(), near)
    };
    let carot = |edits| ("carot".to_owned(), edits);
    assert_eq!(ask("carots", &[]), (0, vec![carot(Edits::One)]));
    assert_eq!(ask("carrots", &[]), (0, Vec::new()));
    assert_eq!(
        ask("carots", &[("BILBO_RECALL_TYPO_TERMS", "0")]),
        (1, Vec::new())
    );
    assert_eq!(
        ask("carots", &[("BILBO_RECALL_TYPO_LETTERS", "6")]),
        (1, Vec::new())
    );
    assert_eq!(
        ask("carots", &[("BILBO_RECALL_TYPO_MAX_LETTERS", "4")]),
        (1, Vec::new())
    );
    assert_eq!(
        ask("carots", &[("BILBO_RECALL_TYPO_TWO_EDITS", "5")]),
        (0, vec![carot(Edits::Two)])
    );
}

#[test]
fn the_pair_weight_key_reaches_the_search() {
    let sandbox = Sandbox::new("recall_pair_weight");
    sandbox.write(
        "notes/pair.md",
        b"Pair: the release checklist covers dates, owners and every other item.\n",
    );
    sandbox.write(
        "notes/twice.md",
        b"Twice: release dates slip, checklist items slip, release owners and checklist owners differ.\n",
    );
    for name in [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
    ] {
        sandbox.write(
            &format!("notes/{name}.md"),
            format!("Apart {name}: release dates slip while the checklist items stay put.\n")
                .as_bytes(),
        );
    }
    sandbox.remember(&["notes"]);
    let ask = |weight: Option<&str>| {
        let seen = Arc::new(Seen::default());
        let set: Vec<_> = weight
            .map(|weight| ("BILBO_RECALL_PAIR_WEIGHT", weight))
            .into_iter()
            .collect();
        let done = run_with_settings(
            &sandbox.settings_with(&set),
            &runner::shared_tokenizer,
            &recording(&seen),
            bilbo_cli::detect(),
            &["release", "checklist"],
        )
        .expect("the run ends");
        let stdout = done.stdout(&View::PLAIN);
        let first = hits(&stdout).remove(0).content;
        let pairs = seen.pairs.lock().expect("pairs").clone();
        (first.split(':').next().map(str::to_owned), pairs)
    };
    let (first, pairs) = ask(Some("0"));
    assert_eq!(first.as_deref(), Some("Twice"), "weight 0");
    assert_eq!(pairs, Vec::<(String, String)>::new());
    let (first, pairs) = ask(None);
    assert_eq!(first.as_deref(), Some("Pair"), "weight unset");
    assert_eq!(pairs, [("releas".to_owned(), "checklist".to_owned())]);
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
fn the_list_limit_key_reaches_the_index_recall_reads() {
    use std::io::Write;
    let sandbox = Sandbox::filled("recall_list_limit_key");
    fs::OpenOptions::new()
        .append(true)
        .open(sandbox.cache().join("index/meta.json"))
        .expect("meta.json")
        .write_all(&vec![b' '; 2 << 20])
        .expect("pad");
    let run = sandbox.run(&["recall", "carrots"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(!hits(&run.stdout).is_empty());
    let run = sandbox.run_with(
        &["recall", "carrots"],
        &[("BILBO_SEARCH_LIST_LIMIT", "1 MiB")],
    );
    assert_eq!((run.code, run.stdout.as_str()), (1, ""));
    assert_eq!(
        run.stderr,
        ["bilbo: the store has no search index bilbo can use; bilbo reindex makes it"]
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
        "bilbo: unknown kind 'nope'; kinds: markdown, session, note"
    );
}

const MAIN_SESSION: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
const SUBAGENT: &str = "agent-ad63c2b3c9e7a4b3d";

fn with_a_session_and_its_subagent(test: &str) -> Sandbox {
    let sandbox = Sandbox::new(test);
    sandbox.write(
        &format!("sessions/claude/p/{MAIN_SESSION}.jsonl"),
        SESSIONS[1].1,
    );
    sandbox.write(
        &format!("sessions/claude/p/{MAIN_SESSION}/subagents/{SUBAGENT}.jsonl"),
        include_bytes!("../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.jsonl"),
    );
    sandbox.write(
        "sessions/claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
        SESSIONS[0].1,
    );
    sandbox.remember(&["sessions"]);
    sandbox
}

fn entry_kept_from(sandbox: &Sandbox, file: &str) -> bilbo_store::id::Id {
    let settings = sandbox.settings();
    let store = Store::open_read_only(&settings.layout(), Options::default(), &mut |_| {})
        .expect("open the store");
    let label =
        bilbo_store::labels::CaptureLabel::file(&sandbox.dir.join(file)).expect("a capture label");
    store
        .by_capture(&label)
        .expect("a lookup")
        .expect("an entry")
        .id
}

#[test]
fn a_scope_by_id_keeps_a_session_and_its_subagents() {
    let sandbox = with_a_session_and_its_subagent("recall_in_id");
    let main = entry_kept_from(&sandbox, &format!("sessions/claude/p/{MAIN_SESSION}.jsonl"));
    let other = entry_kept_from(
        &sandbox,
        "sessions/claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
    );
    for value in [main.to_string(), main.short().to_string()] {
        let run = sandbox.run(&["recall", "--in", &value, "seed", "packets"]);
        assert_eq!(run.code, 0, "{value}: {:?}", run.stderr);
        assert!(
            run.stderr
                .iter()
                .any(|line| line.starts_with("bilbo: --in kept 2 entries and left out ")),
            "{:?}",
            run.stderr
        );
        let found = hits(&run.stdout);
        assert!(
            found.iter().any(|hit| hit.content.contains("seed packets")),
            "{}",
            run.stdout
        );
        let subagent_path = format!("{MAIN_SESSION}/subagents/{SUBAGENT}.jsonl");
        assert!(
            found
                .iter()
                .any(|hit| capture_of(&sandbox, hit).ends_with(&subagent_path)),
            "{}",
            run.stdout
        );
    }
    let outside = sandbox.run(&["recall", "--in", &other.to_string(), "seed", "packets"]);
    for hit in hits(&outside.stdout) {
        assert!(
            capture_of(&sandbox, &hit).ends_with("8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"),
            "{}",
            outside.stdout
        );
    }
}

#[test]
fn a_scope_by_a_word_of_the_capture_label_keeps_those_entries() {
    let sandbox = Sandbox::new("recall_in_words");
    sandbox.write(
        "notes/greenhouse/a.md",
        b"# Beds\n\nSeville marmalade ripens beside the tomato vines, forty jars by October.\n",
    );
    sandbox.write(
        "notes/pantry/b.md",
        b"# Shelf\n\nQuince marmalade keeps on the top rack behind the flour bins, away from damp.\n",
    );
    sandbox.remember(&["notes"]);
    let all = sandbox.run(&["recall", "marmalade"]);
    assert_eq!(hits(&all.stdout).len(), 2, "{}", all.stdout);
    for (word, folder) in [("greenhouse", "greenhouse/a.md"), ("pantry", "pantry/b.md")] {
        let run = sandbox.run(&["recall", "--in", word, "marmalade"]);
        assert_eq!(run.code, 0, "{word}: {:?}", run.stderr);
        let found = hits(&run.stdout);
        assert_eq!(found.len(), 1, "{}", run.stdout);
        assert!(capture_of(&sandbox, &found[0]).ends_with(folder));
        assert!(
            run.stderr
                .iter()
                .any(|line| line.starts_with("bilbo: --in kept 1 entry")),
            "{:?}",
            run.stderr
        );
    }
}

#[test]
fn a_folder_with_an_accent_is_named_without_it() {
    let sandbox = Sandbox::new("recall_in_accents");
    sandbox.write(
        "notes/reunia\u{303}o/a.md",
        b"# Beds\n\nSeville marmalade ripens beside the tomato vines, forty jars by October.\n",
    );
    sandbox.write(
        "notes/pantry/b.md",
        b"# Shelf\n\nQuince marmalade keeps on the top rack behind the flour bins, away from damp.\n",
    );
    sandbox.remember(&["notes"]);
    for value in ["reuniao", "reuni\u{e3}o", "reunia\u{303}o"] {
        let run = sandbox.run(&["recall", "--in", value, "marmalade"]);
        assert_eq!(run.code, 0, "{value:?}: {:?}", run.stderr);
        let found = hits(&run.stdout);
        assert_eq!(found.len(), 1, "{value:?}: {}", run.stdout);
        assert!(capture_of(&sandbox, &found[0]).contains("reunia"));
        assert!(
            run.stderr
                .iter()
                .any(|line| line.starts_with("bilbo: --in kept 1 entry")),
            "{value:?}: {:?}",
            run.stderr
        );
    }
}

const WORKED: &str = "7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d";
const ROLLOUT: &str = "rollout-2026-11-08T10-00-00-019a6c4d-3e5f-7a71-9b2c-4d6e8fa0b1c3";

#[test]
fn a_scope_by_a_folder_a_session_worked_in_keeps_it() {
    let sandbox = Sandbox::new("recall_in_folder");
    sandbox.write(
        &format!("sessions/claude/p/{WORKED}.jsonl"),
        include_bytes!(
            "../../../libs/entries/rust/fixtures/session/7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl"
        ),
    );
    sandbox.write(
        &format!("sessions/codex/2026/11/08/{ROLLOUT}.jsonl"),
        include_bytes!(
            "../../../libs/entries/rust/fixtures/session/rollout-2026-11-08T10-00-00-019a6c4d-3e5f-7a71-9b2c-4d6e8fa0b1c3.jsonl"
        ),
    );
    sandbox.write(FIRST_SESSION, SESSIONS[0].1);
    sandbox.remember(&["sessions"]);
    for value in ["--in=-home-me-src-shed", "--in=/home/me/src/shed"] {
        let run = sandbox.run(&["recall", value, "frost", "compost", "heap", "greenhouse"]);
        assert_eq!(run.code, 0, "{value}: {:?}", run.stderr);
        assert!(
            run.stderr
                .iter()
                .any(|line| line.starts_with("bilbo: --in kept 2 entries")),
            "{value}: {:?}",
            run.stderr
        );
        let captures: Vec<String> = hits(&run.stdout)
            .iter()
            .map(|hit| capture_of(&sandbox, hit))
            .collect();
        assert!(!captures.is_empty(), "{value}: {}", run.stdout);
        for capture in &captures {
            assert!(
                capture.ends_with(&format!("{WORKED}.jsonl"))
                    || capture.ends_with(&format!("{ROLLOUT}.jsonl")),
                "{value}: {capture}"
            );
        }
    }
    let one = sandbox.run(&["recall", "--in", "/home/me/src/greenhouse", "frost"]);
    assert_eq!(one.code, 0, "{:?}", one.stderr);
    assert!(
        one.stderr
            .iter()
            .any(|line| line.starts_with("bilbo: --in kept 1 entry")),
        "{:?}",
        one.stderr
    );
}

#[test]
fn a_value_with_no_letter_or_digit_is_a_usage_error() {
    let sandbox = Sandbox::filled("recall_in_blank");
    let before = sandbox.tree(false);
    for given in [&["--in", "."][..], &["--in=--"]] {
        let mut args = vec!["recall"];
        args.extend_from_slice(given);
        args.push("carrots");
        let run = sandbox.run(&args);
        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{given:?}");
        let value = given.last().map(|value| value.trim_start_matches("--in="));
        assert_eq!(
            run.stderr[0],
            format!(
                "bilbo: --in '{}' has no letter or digit; give a word of a label, or an id as a hit prints it",
                value.expect("a value")
            )
        );
        assert!(
            run.stderr[1].starts_with("bilbo: usage: bilbo recall"),
            "{:?}",
            run.stderr
        );
    }
    assert!(!sandbox.log().exists(), "a usage error is not logged");
    assert!(
        sandbox.tree(false) == before,
        "the store, state or cache changed"
    );
}

#[test]
fn in_with_no_value_needs_a_value() {
    let sandbox = Sandbox::filled("recall_in_empty");
    for args in [
        &["recall", "--in=", "carrots"][..],
        &["recall", "carrots", "--in"],
        &["recall", "--in", "-home-me-src-shed", "carrots"],
    ] {
        let run = sandbox.run(args);
        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{args:?}");
        assert_eq!(run.stderr[0], "bilbo: --in needs a value", "{args:?}");
    }
}

#[test]
fn recall_with_a_scope_writes_no_file_but_the_query_log() {
    let sandbox = Sandbox::filled("recall_in_writes");
    let before = sandbox.tree(false);
    let queries: [(&[&str], u8, &str); 3] = [
        (&["recall", "--in", "notes", "carrots"], 0, "hits"),
        (&["recall", "--in", "claude", "xyzzy"], 1, "nothing"),
        (&["recall", "--in", "nowhere", "carrots"], 1, "nothing"),
    ];
    for (args, code, _) in queries {
        let run = sandbox.run(args);
        assert_eq!(run.code, code, "{args:?}: {:?}", run.stderr);
    }
    assert!(
        sandbox.tree(true) == before,
        "a scoped recall changed the store, the state or the cache"
    );
    let log = fs::read_to_string(sandbox.log()).expect("the query log");
    let fields: Vec<Vec<&str>> = log.lines().map(|line| line.split('\t').collect()).collect();
    assert_eq!(fields.len(), 3, "{log}");
    assert_eq!(
        fields
            .iter()
            .map(|line| (line[0], line[5]))
            .collect::<Vec<_>>(),
        [("q3", "notes"), ("q3", "claude"), ("q3", "nowhere")]
    );
    assert_eq!(fields[2][6], "0", "a value that named nothing: {log}");
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
    fs::write(
        sandbox.dir.join("config/config.toml"),
        "[recall]\nquestion_words = [\"n\u{e3}o\"]\n",
    )
    .expect("a config file");
    let refused = sandbox.run(&["recall", "anything"]);
    assert_eq!(refused.code, 2, "{:?}", refused.stderr);
    assert!(
        refused.stderr.iter().any(|line| line.contains(
            "recall.question_words must be a list of words as the analyzer reads them: lowercase, with no accent or other mark, no compatibility form it folds, and Hangul as jamo, got \"n\u{e3}o\""
        )),
        "{:?}",
        refused.stderr
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
    run_with_settings(&sandbox.settings(), loader, lookup, detect, query)
}

fn run_with_settings(
    settings: &Settings,
    loader: &dyn Fn() -> Result<Tokenizer, Failure>,
    lookup: &dyn Fn(&std::path::Path) -> Box<dyn bilbo_search::lookup::Lookup>,
    detect: &dyn bilbo_search::analyze::Detect,
    query: &[&str],
) -> Result<bilbo_cli::console::outcome::Outcome, Failure> {
    let readers = bilbo_cli::readers(settings).expect("the readers");
    let render = bilbo_cli::rendering(settings).expect("the rendering");
    let reading = bilbo_cli::console::reading::Reading {
        readers: &readers,
        chunk: settings.search.chunk(),
        render: &render,
        tokenizer: loader,
        lookup,
        detect,
        apart: &[],
    };
    let args = bilbo_cli::recall::Args {
        query: query.iter().map(|word| (*word).to_owned()).collect(),
        scope: Vec::new(),
        kind: Vec::new(),
        budget: None,
    };
    bilbo_cli::recall::run(
        &bilbo_cli::bilbo(),
        &args,
        &mut Env::quiet(),
        settings,
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
            bilbo_cli::lookup(path, &bilbo_search::keys::defaults())
        };
        let outcome = run_with_reading(&sandbox, &runner::shared_tokenizer, &lookup);
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
    let outcome = run_with_reading(&sandbox, &other, &|path| {
        bilbo_cli::lookup(path, &bilbo_search::keys::defaults())
    })
    .expect("an outcome");
    assert_eq!(outcome.exit_code(), 1);
    assert_eq!(
        outcome.stderr_lines(&View::PLAIN),
        [
            "bilbo: the search index was made with other settings or by another bilbo; bilbo reindex makes it again"
        ]
    );
    let real = run_with_reading(&sandbox, &runner::shared_tokenizer, &|path| {
        bilbo_cli::lookup(path, &bilbo_search::keys::defaults())
    })
    .expect("an outcome");
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
        bilbo_cli::lookup(path, &bilbo_search::keys::defaults())
    };
    let outcome =
        run_with_reading(&sandbox, &runner::shared_tokenizer, &lookup).expect("an outcome");
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
        let outcome =
            run_with_reading(&sandbox, &runner::shared_tokenizer, &lookup).expect("an outcome");
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
            found.iter().any(|hit| meta_of(&sandbox, hit).kind == kind),
            "no {kind} hit without --kind"
        );
        let narrowed = sandbox.run(&["recall", "--kind", kind, "carrots"]);
        assert_eq!(narrowed.code, 0, "{:?}", narrowed.stderr);
        let only = hits(&narrowed.stdout);
        assert!(!only.is_empty());
        assert!(
            only.iter().all(|hit| meta_of(&sandbox, hit).kind == kind),
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

const SMALL_CHUNKS: (&str, &str) = ("BILBO_SEARCH_CHUNK_TOKENS", "256");

fn words_around(word: &str, count: usize) -> String {
    (0..count)
        .map(|n| {
            if n % 8 == 0 {
                word.to_owned()
            } else {
                format!("zq{n}x")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn remember_with(sandbox: &Sandbox, folder: &str, extra: &[(&str, &str)]) {
    let run = sandbox.run_with(&["remember", &sandbox.path(folder)], extra);
    assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
}

fn passed_over(run: &Run) -> Option<&String> {
    run.stderr
        .iter()
        .find(|line| line.starts_with("bilbo: passed over: "))
}

#[test]
fn the_same_note_in_two_files_is_recalled_once() {
    let sandbox = Sandbox::new("recall_copies");
    let note = "# Beds\n\nThe turnips grew well in the east bed this year.\n";
    sandbox.write("notes/a.md", note.as_bytes());
    sandbox.write("notes/b.md", note.as_bytes());
    sandbox.remember(&["notes"]);
    let run = sandbox.run(&["recall", "turnips"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(hits(&run.stdout).len(), 1, "{}", run.stdout);
    assert_eq!(
        passed_over(&run).map(String::as_str),
        Some("bilbo: passed over: 1 copy")
    );
    let both = sandbox.run_with(&["recall", "turnips"], &[("BILBO_RECALL_COPY_SHARE", "0")]);
    assert_eq!(hits(&both.stdout).len(), 2, "{}", both.stdout);
    assert_eq!(passed_over(&both), None);
}

#[test]
fn the_copy_and_record_keys_reach_the_search() {
    let sandbox = Sandbox::new("recall_keys");
    let line = words_around("turnips", 150);
    sandbox.write("notes/long.md", format!("# Long\n\n{line}\n").as_bytes());
    remember_with(&sandbox, "notes", &[SMALL_CHUNKS]);
    let shown = |extra: &[(&str, &str)]| {
        let mut vars = vec![SMALL_CHUNKS];
        vars.extend_from_slice(extra);
        let run = sandbox.run_with(&["recall", "turnips"], &vars);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        hits(&run.stdout).len()
    };
    assert_eq!(shown(&[]), 2);
    assert_eq!(shown(&[("BILBO_RECALL_RECORD_CAP", "0")]), 4);
    assert_eq!(shown(&[("BILBO_RECALL_RECORD_CAP", "1")]), 1);
}

#[test]
fn a_long_section_of_several_lines_shows_no_line_in_more_than_two_hits() {
    let sandbox = Sandbox::new("recall_record_cap");
    let middle = words_around("turnips", 150);
    sandbox.write(
        "notes/section.md",
        format!("# Beds\n\nturnips come first\n{middle}\nturnips end it\n").as_bytes(),
    );
    remember_with(&sandbox, "notes", &[SMALL_CHUNKS]);
    let run = sandbox.run_with(&["recall", "turnips"], &[SMALL_CHUNKS]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let mut held: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    for hit in hits(&run.stdout) {
        let records = Ref::parse(&hit.reference)
            .expect("a ref")
            .locator
            .records
            .expect("records");
        for record in records.first..records.first + records.count {
            *held.entry(record).or_default() += 1;
        }
    }
    assert!(!held.is_empty());
    assert!(held.values().all(|count| *count <= 2), "{held:?}");
    assert!(
        passed_over(&run).is_some_and(|line| line.contains("further part")),
        "{:?}",
        run.stderr
    );
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
            &[
                ("BILBO_RECALL_BM25_B", "0"),
                ("BILBO_RECALL_BM25_K1", k1),
                ("BILBO_RECALL_PAIR_WEIGHT", "0"),
            ],
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
        let head = bilbo_store::labels::CaptureLabel::file(&path)
            .and_then(|label| store.by_capture(&label).expect("a lookup"));
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
        Some(Language::of("pt").expect("a Portuguese language"))
    }
}

#[derive(Default)]
struct Seen {
    terms: Mutex<BTreeSet<String>>,
    near: Mutex<Vec<(String, Edits)>>,
    pairs: Mutex<Vec<(String, String)>>,
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

    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
        self.seen
            .near
            .lock()
            .expect("near")
            .push((term.to_owned(), edits));
        self.inner.near(term, edits)
    }

    fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
        self.seen
            .pairs
            .lock()
            .expect("pairs")
            .push((first.to_owned(), second.to_owned()));
        self.inner.pairs(first, second)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.inner.holding(filter)
    }

    fn length(&self, kind: &str) -> Result<u64, IndexError> {
        self.inner.length(kind)
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
            inner: bilbo_cli::lookup(path, &bilbo_search::keys::defaults()),
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
            &runner::shared_tokenizer,
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
        Some(Language::of("pt").expect("a Portuguese language"))
    );
    assert_eq!(detect.language("Hello, happy tax payer"), None);
}

const FILLERS: [&str; 8] = [
    "The thinning left a hand's width between plants and the second sowing was set for March with zucchini beside them.",
    "Radishes went into the cold frame beside the shed while the compost heap was turned and covered with straw for winter.",
    "Pole beans climbed the old fence after the neighbour lent a ladder, and the wet corner drained once a trench was cut.",
    "Lettuce seedlings were hardened off on the porch each afternoon before moving out under the cloth tunnel at dusk.",
    "Garlic cloves were pushed in rows near the herb spiral, mulched thickly, and marked with painted sticks along the path.",
    "The rain barrel overflowed twice during the storm so a second hose was fitted to carry water down toward the pond.",
    "Tomato plants outgrew their cages, which prompted a trip for taller stakes and soft ties made from strips of old cotton.",
    "Strawberry runners were pinned into small pots filled with sand and loam, then kept shaded until roots had taken hold.",
];

fn note_line(parent: bilbo_store::id::Id, number: u64, sources: &[(u64, u64)]) -> Vec<u8> {
    note_line::line(
        parent,
        &format!(
            "The user asked about the carrot rows, number {number}. {}",
            FILLERS[number as usize % FILLERS.len()]
        ),
        Some(1_794_039_249),
        sources,
    )
}

struct Noted {
    sandbox: Sandbox,
    parent: bilbo_store::id::Id,
    note: bilbo_store::id::Id,
    before: Before,
}

struct Before {
    plain: Run,
    sessions: Run,
    scoped: Run,
}

fn with_notes(test: &str, count: u64) -> Noted {
    let sandbox = Sandbox::filled(test);
    let parent = entry_kept_from(&sandbox, FIRST_SESSION);
    let before = Before {
        plain: sandbox.run(&["recall", "carrot", "rows"]),
        sessions: sandbox.run(&["recall", "--kind", "session", "carrot", "rows"]),
        scoped: sandbox.run(&[
            "recall",
            "--in",
            &parent.short().to_string(),
            "carrot",
            "rows",
        ]),
    };
    let lines: Vec<Vec<u8>> = (1..=count)
        .map(|number| {
            note_line(
                parent,
                number,
                &[(3, 3), (5, 6)][..(1 + (number % 2)) as usize],
            )
        })
        .collect();
    let note = notes::noted(&sandbox.settings().layout(), parent, &lines);
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    Noted {
        sandbox,
        parent,
        note,
        before,
    }
}

fn is_note(hit: &Hit) -> bool {
    hit.extras.first().is_some_and(|extra| extra == "note")
}

fn notes_part(stdout: &str) -> (String, String) {
    let at = stdout
        .lines()
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len() + 1;
            Some((start, line))
        })
        .find(|(_, line)| line.starts_with("=== ") && line.ends_with("\tnote"))
        .map_or(stdout.len(), |(start, _)| start);
    (stdout[..at].to_owned(), stdout[at..].to_owned())
}

fn note_text(noted: &Noted, number: u64, sources: &str) -> String {
    format!(
        "=== {}#r{number}\t2 lines\tnote\n{}\nThe user asked about the carrot rows, number {number}. {}\n",
        noted.note.short(),
        sources,
        FILLERS[number as usize % FILLERS.len()]
    )
}

fn notes_printed(noted: &Noted) -> [String; 2] {
    let line = format!("{}#g1:r3", noted.parent);
    [
        note_text(noted, 2, &format!("{line} 2026-11-07")),
        note_text(
            noted,
            1,
            &format!("{line} {}#g1:r5-6 2026-11-07", noted.parent),
        ),
    ]
}

#[test]
fn a_store_with_notes_answers_the_chunks_then_the_notes() {
    let noted = with_notes("recall_notes", 2);
    let run = noted.sandbox.run(&["recall", "carrot", "rows"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let expected = format!(
        "{}{}",
        noted.before.plain.stdout,
        notes_printed(&noted).concat()
    );
    assert_eq!(run.stdout, expected);
    let count = |run: &Run| -> u64 {
        run.stderr
            .iter()
            .find_map(|line| {
                line.strip_prefix("bilbo: searched ")?
                    .strip_suffix(" entries")?
                    .parse()
                    .ok()
            })
            .expect("a count")
    };
    assert_eq!(count(&run), count(&noted.before.plain) + 1);
    let found = hits(&run.stdout);
    let reference = Ref::parse(&found[4].reference).expect("a ref");
    let store = Store::open_read_only(
        &noted.sandbox.settings().layout(),
        Options::default(),
        &mut |_| {},
    )
    .expect("open the store");
    assert_eq!(entry_of(&store, &reference), noted.note);
}

#[test]
fn a_word_only_a_note_holds_leaves_the_chunks_as_they_were() {
    let sandbox = Sandbox::filled("recall_notes_typo");
    let parent = entry_kept_from(&sandbox, FIRST_SESSION);
    let before = sandbox.run(&["recall", "carot", "rows"]);
    assert_eq!(before.code, 0, "{:?}", before.stderr);
    assert!(before.stderr.iter().any(|line| line.contains("typos:")));
    let text = "The user wrote carot where the carrot rows were meant.";
    let line = format!(
        "{{\"text\":\"{text}\",\"at\":1794039249,\"from\":[{{\"entry\":\"{parent}\",\"generation\":1,\"records\":[3,3]}}]}}\n"
    );
    let note = notes::noted(&sandbox.settings().layout(), parent, &[line.into_bytes()]);
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let after = sandbox.run(&["recall", "carot", "rows"]);
    assert_eq!(after.code, 0, "{:?}", after.stderr);
    assert_eq!(
        after.stdout,
        format!(
            "{}=== {}#r1\t2 lines\tnote\n{parent}#g1:r3 2026-11-07\n{text}\n",
            before.stdout,
            note.short()
        )
    );
    let typos = |run: &Run| -> Vec<String> {
        run.stderr
            .iter()
            .filter(|line| line.contains("typos:"))
            .cloned()
            .collect()
    };
    assert_eq!(typos(&after), typos(&before));
}

#[test]
fn the_budget_flag_bounds_the_chunks_and_the_key_bounds_the_notes() {
    let noted = with_notes("recall_notes_budget", 6);
    let base = noted.sandbox.run(&["recall", "carrot", "rows"]);
    let (chunks, notes) = notes_part(&base.stdout);
    assert!(!chunks.is_empty() && !notes.is_empty());
    let flagged = noted
        .sandbox
        .run(&["recall", "--budget", "300", "carrot", "rows"]);
    let (flagged_chunks, flagged_notes) = notes_part(&flagged.stdout);
    assert_ne!(flagged_chunks, chunks);
    assert_eq!(flagged_notes, notes);
    let keyed = noted.sandbox.run_with(
        &["recall", "carrot", "rows"],
        &[("BILBO_RECALL_NOTES_BUDGET", "256")],
    );
    let (keyed_chunks, keyed_notes) = notes_part(&keyed.stdout);
    assert_eq!(keyed_chunks, chunks);
    let cost = |part: &str| -> usize {
        hits(part)
            .iter()
            .map(|hit| Shared.count(&hit.printed))
            .sum()
    };
    assert!(!hits(&keyed_notes).is_empty(), "{keyed_notes}");
    assert!(cost(&keyed_notes) <= 256, "{}", cost(&keyed_notes));
    assert!(cost(&notes) > 256, "{}", cost(&notes));
}

#[test]
fn a_notes_budget_of_zero_prints_no_note_and_no_note_among_the_chunks() {
    let noted = with_notes("recall_notes_zero", 3);
    let base = noted.sandbox.run(&["recall", "carrot", "rows"]);
    let zero = noted.sandbox.run_with(
        &["recall", "carrot", "rows"],
        &[("BILBO_RECALL_NOTES_BUDGET", "0")],
    );
    assert_eq!(zero.code, 0, "{:?}", zero.stderr);
    assert!(!entries_named(&noted.sandbox, &zero).contains(&noted.note));
    assert_eq!(zero.stdout, notes_part(&base.stdout).0);
}

#[test]
fn kind_note_prints_the_notes_alone_and_another_kind_prints_none() {
    let noted = with_notes("recall_notes_kind", 2);
    let printed = |args: &[&str]| -> String {
        let mut all = vec!["recall"];
        all.extend_from_slice(args);
        all.extend(["carrot", "rows"]);
        let run = noted.sandbox.run(&all);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        run.stdout
    };
    let notes = notes_printed(&noted).concat();
    assert_eq!(printed(&["--kind", "note"]), notes);
    assert_eq!(
        printed(&["--kind", "session"]),
        noted.before.sessions.stdout
    );
    assert_eq!(
        printed(&["--kind", "note", "--kind", "session"]),
        format!("{}{}", noted.before.sessions.stdout, notes)
    );
}

#[test]
fn in_an_entry_brings_its_notes_and_counts_them() {
    let noted = with_notes("recall_notes_in", 2);
    let run = noted.sandbox.run(&[
        "recall",
        "--in",
        &noted.parent.short().to_string(),
        "carrot",
        "rows",
    ]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let counted = |line: &String| -> String {
        let bump = |text: &str, prefix: &str| -> Option<String> {
            let rest = text.strip_prefix(prefix)?;
            let (number, tail) = rest.split_once(' ')?;
            let number = number.parse::<u64>().ok()? + 1;
            let tail = match tail.strip_prefix("entry") {
                Some(rest) if number != 1 => format!("entries{rest}"),
                _ => tail.to_owned(),
            };
            Some(format!("{prefix}{number} {tail}"))
        };
        bump(line, "bilbo: searched ")
            .or_else(|| bump(line, "bilbo: --in kept "))
            .unwrap_or_else(|| line.clone())
    };
    let expected: Vec<String> = noted.before.scoped.stderr.iter().map(counted).collect();
    assert_eq!(run.stderr, expected);
    assert_eq!(
        run.stdout,
        format!(
            "{}{}",
            noted.before.scoped.stdout,
            notes_printed(&noted).concat()
        )
    );
    let other = noted
        .sandbox
        .run(&["recall", "--in", "codex", "carrot", "rows"]);
    assert!(!hits(&other.stdout).iter().any(is_note), "{}", other.stdout);
}

#[test]
fn a_notes_source_line_opens_the_records_it_names() {
    let noted = with_notes("recall_notes_source", 2);
    let run = noted
        .sandbox
        .run(&["recall", "--kind", "note", "carrot", "rows"]);
    let found = hits(&run.stdout);
    assert!(found.len() >= 2, "{}", run.stdout);
    for hit in &found {
        let line = hit.content.lines().next().expect("a source line");
        let sources: Vec<&str> = line
            .split(' ')
            .take_while(|part| part.contains('#'))
            .collect();
        assert!(!sources.is_empty(), "{line}");
        for source in sources {
            let (_, records) = source.split_once(":r").expect("a locator");
            let first = records.split('-').next().expect("a first record");
            let read = noted.sandbox.run(&["read", source]);
            assert_eq!(read.code, 0, "{source}: {:?}", read.stderr);
            assert!(
                read.stdout.starts_with(&format!("#r{first}")),
                "{source}: {}",
                read.stdout
            );
        }
    }
    assert!(found.iter().any(|hit| {
        hit.content
            .lines()
            .next()
            .is_some_and(|line| line.matches('#').count() == 2)
    }));
}

#[test]
fn a_note_printed_without_its_source_line_is_passed_over_as_a_copy() {
    let sandbox = Sandbox::new("recall_notes_copy");
    sandbox.write("notes/garden.md", b"alpha beta gamma delta epsilon zeta\n");
    sandbox.remember(&["notes"]);
    let parent = entry_kept_from(&sandbox, "notes/garden.md");
    let sources: Vec<(u64, u64)> = (1..=100).map(|n| (n, n)).collect();
    let line = note_line::line(
        parent,
        "alpha beta gamma delta epsilon zeta eta",
        Some(1_794_039_249),
        &sources,
    );
    notes::noted(&sandbox.settings().layout(), parent, &[line]);
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let run = sandbox.run(&[
        "recall", "alpha", "beta", "gamma", "delta", "epsilon", "zeta",
    ]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(!hits(&run.stdout).is_empty(), "{}", run.stdout);
    assert!(!hits(&run.stdout).iter().any(is_note), "{}", run.stdout);
    assert!(
        run.stderr
            .contains(&"bilbo: passed over: 1 copy".to_owned()),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_long_note_is_printed_in_parts_under_its_whole_source_line_or_under_none() {
    let sandbox = Sandbox::filled("recall_notes_long");
    let parent = entry_kept_from(&sandbox, FIRST_SESSION);
    let sentences: Vec<String> = (1..=40)
        .map(|n| match n {
            1 | 40 => format!("Sentence {n} names the kohlrabi by the south fence."),
            _ => format!("Sentence {n} says the beans were tied up and the compost turned."),
        })
        .collect();
    let paragraph = sentences.join(" ");
    let single = "One sentence names the kohlrabi.";
    let long = format!(
        "{{\"text\":\"{paragraph}\",\"at\":1794039249,\"from\":[{{\"entry\":\"{parent}\",\"generation\":1,\"records\":[3,3]}}]}}\n"
    );
    let many: Vec<String> = (1..=100)
        .map(|n| format!("{{\"entry\":\"{parent}\",\"generation\":1,\"records\":[{n},{n}]}}"))
        .collect();
    let crowded = format!(
        "{{\"text\":\"{single}\",\"at\":1794039249,\"from\":[{}]}}\n",
        many.join(",")
    );
    let note = notes::noted(
        &sandbox.settings().layout(),
        parent,
        &[long.into_bytes(), crowded.into_bytes()],
    );
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let run = sandbox.run_with(
        &["recall", "--kind", "note", "kohlrabi"],
        &[
            ("BILBO_RECALL_RECORD_CAP", "0"),
            ("BILBO_RECALL_NOTES_BUDGET", "20000"),
        ],
    );
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let found = hits(&run.stdout);
    let first_record = found
        .iter()
        .filter(|hit| hit.reference.ends_with("#r1"))
        .collect::<Vec<_>>();
    assert!(first_record.len() >= 2, "{}", run.stdout);
    assert!(
        first_record
            .iter()
            .any(|hit| hit.extras.iter().any(|extra| extra.starts_with("part 1/"))),
        "{}",
        run.stdout
    );
    for hit in &first_record {
        assert_eq!(
            hit.content.lines().next(),
            Some(format!("{parent}#g1:r3 2026-11-07").as_str()),
            "{}",
            hit.printed
        );
    }
    let second: Vec<&Hit> = found
        .iter()
        .filter(|hit| hit.reference.ends_with("#r2"))
        .collect();
    assert_eq!(second.len(), 1, "{}", run.stdout);
    assert_eq!(second[0].extras, ["note", "part 1/1"]);
    assert_eq!(second[0].content, single);
    assert!(!second[0].content.lines().any(|line| line.contains("#g")));
    let read = sandbox.run(&["read", &format!("{}#r2", note.short())]);
    assert_eq!(read.code, 0, "{:?}", read.stderr);
    let line = read
        .stdout
        .lines()
        .find(|line| line.contains("#g1:r1"))
        .expect("the source line");
    assert_eq!(line.matches("#g1:r").count(), 100, "{line}");
}

#[test]
fn a_note_alone_is_a_hit_and_exit_0() {
    let noted = with_notes("recall_notes_alone", 1);
    let run = noted.sandbox.run(&["recall", "radishes"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let found = hits(&run.stdout);
    assert!(
        !found.is_empty() && found.iter().all(is_note),
        "{}",
        run.stdout
    );
    assert_eq!(run.stderr, ["bilbo: searched 6 entries"]);
}

#[test]
fn two_short_notes_of_one_window_are_both_printed() {
    let sandbox = Sandbox::filled("recall_notes_short");
    let parent = entry_kept_from(&sandbox, FIRST_SESSION);
    let lines = [
        "The user asked to thin the rows.",
        "The user asked to water the beds.",
    ]
    .map(|text| note_line::line(parent, text, Some(1_794_039_249), &[(3, 3)]));
    let note = notes::noted(&sandbox.settings().layout(), parent, &lines);
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let run = sandbox.run(&["recall", "user", "asked"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let printed = |number: u64, text: &str| {
        format!(
            "=== {}#r{number}\t2 lines\tnote\n{parent}#g1:r3 2026-11-07\n{text}\n",
            note.short()
        )
    };
    assert_eq!(
        notes_part(&run.stdout).1,
        format!(
            "{}{}",
            printed(1, "The user asked to thin the rows."),
            printed(2, "The user asked to water the beds.")
        )
    );
}

#[test]
fn a_note_with_no_source_prints_an_empty_first_line() {
    let sandbox = Sandbox::filled("recall_notes_no_source");
    let parent = entry_kept_from(&sandbox, FIRST_SESSION);
    let line = note_line::line(
        parent,
        "01M6ZGE4KTBYYH07AS2KVMSCAF#g1:r3-19 2026-11-07\\nThe beds are done.",
        None,
        &[],
    );
    let note = notes::noted(&sandbox.settings().layout(), parent, &[line]);
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let run = sandbox.run(&["recall", "--kind", "note", "beds"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "=== {}#r1\t3 lines\tnote\n\n01M6ZGE4KTBYYH07AS2KVMSCAF#g1:r3-19 2026-11-07\nThe beds are done.\n",
            note.short()
        )
    );
}

#[test]
fn a_scope_brings_the_notes_of_the_entry_it_names_and_no_other() {
    let sandbox = Sandbox::filled("recall_notes_scope");
    let first = entry_kept_from(&sandbox, FIRST_SESSION);
    let second = entry_kept_from(
        &sandbox,
        "sessions/claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl",
    );
    let texts = [
        "The user asked about the kohlrabi by the fence.",
        "The user asked about the kohlrabi by the pond.",
    ];
    let layout = sandbox.settings().layout();
    let notes = [first, second].map(|parent| {
        (
            parent,
            notes::noted(
                &layout,
                parent,
                &[note_line::line(
                    parent,
                    texts[usize::from(parent == second)],
                    Some(1_794_039_249),
                    &[(3, 3)],
                )],
            ),
        )
    });
    let run = sandbox.run(&["reindex"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    for (parent, note) in notes {
        let run = sandbox.run(&["recall", "--in", &parent.short().to_string(), "kohlrabi"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        let text = texts[usize::from(parent == second)];
        assert_eq!(
            notes_part(&run.stdout).1,
            format!(
                "=== {}#r1\t2 lines\tnote\n{parent}#g1:r3 2026-11-07\n{text}\n",
                note.short()
            )
        );
        assert_eq!(
            run.stderr,
            ["bilbo: searched 7 entries", "bilbo: --in kept 2 entries"]
        );
    }
}
