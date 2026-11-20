use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::{Env, Var};
use bilbo_cli::console::indexing::Indexing;
use bilbo_cli::console::settings;
use bilbo_derive::instruction;
use bilbo_derive::tries::Extent as Charged;
use bilbo_provider::{Provider, Reply, Request};
use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats};
use bilbo_search::position::Position;
use bilbo_store::entries::{Capture, Cursor, Entries, Generation, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::labels::CaptureLabel;
use bilbo_store::segments::{Options, Store};
use serde_json::{Value, json};

mod notes;
mod runner;
mod scratch;
mod stores;

const KEY: &str = "sk-test-PLUM-7391";
const FIRST: &str = "8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b";
const SECOND: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
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
const SUBAGENT: [(&str, &[u8]); 2] = [
    (
        "claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03/subagents/agent-ad63c2b3c9e7a4b3d.jsonl",
        include_bytes!("../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.jsonl"),
    ),
    (
        "claude/p/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03/subagents/agent-ad63c2b3c9e7a4b3d.meta.json",
        include_bytes!(
            "../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.meta.json"
        ),
    ),
];
const NOTES: [(&str, &[u8]); 1] = [(
    "sections.md",
    include_bytes!("../../../libs/entries/rust/fixtures/markdown/sections.md"),
)];
const CLOSED: &str = "http://127.0.0.1:9/";

// A child process holds every open file of this process until it executes, another test's store lock included.
static RUNS: RwLock<()> = RwLock::new(());

thread_local! {
    static REFUSING: Cell<bool> = const { Cell::new(false) };
    static HELD: Cell<bool> = const { Cell::new(false) };
}

struct Together {
    _guard: RwLockReadGuard<'static, ()>,
}

struct Alone {
    _guard: RwLockWriteGuard<'static, ()>,
}

fn together() -> Together {
    let _guard = RUNS.read().unwrap_or_else(PoisonError::into_inner);
    HELD.with(|flag| flag.set(true));
    Together { _guard }
}

fn alone() -> Alone {
    let _guard = RUNS.write().unwrap_or_else(PoisonError::into_inner);
    HELD.with(|flag| flag.set(true));
    Alone { _guard }
}

fn held_by_the_test() {
    HELD.with(|flag| flag.set(true));
}

impl Drop for Together {
    fn drop(&mut self) {
        HELD.with(|flag| flag.set(false));
    }
}

impl Drop for Alone {
    fn drop(&mut self) {
        HELD.with(|flag| flag.set(false));
    }
}

struct Refusing(Box<dyn Index>);

impl Index for Refusing {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        self.0.position()
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.0.clear()
    }

    fn add(&mut self, doc: &Doc<'_>) -> Result<(), IndexError> {
        self.0.add(doc)
    }

    fn commit(&mut self, _: &Position) -> Result<Stats, IndexError> {
        self.0.rollback();
        Err(IndexError::Failed {
            action: "commit",
            path: PathBuf::from("/refused"),
            error: "refused".to_owned(),
        })
    }

    fn extents(&mut self, entry: Id) -> Result<Vec<Extent>, IndexError> {
        self.0.extents(entry)
    }

    fn delete(&mut self, entry: Id, from: u64) -> Result<(), IndexError> {
        self.0.delete(entry, from)
    }

    fn rollback(&mut self) {
        self.0.rollback();
    }
}

fn tune(mut indexing: Indexing<'_>) -> Indexing<'_> {
    indexing.tokenizer = &runner::shared_tokenizer;
    if REFUSING.with(Cell::get) {
        let inner = indexing.index;
        indexing.index = Box::leak(Box::new(move |cache: &Path| -> Box<dyn Index> {
            Box::new(Refusing(inner(cache)))
        }));
    }
    indexing
}

use runner::{Run, Sandbox};

fn bilbo(vars: HashMap<String, OsString>, args: &[&str]) -> Run {
    bilbo_with(Rc::new(move |name| vars.get(name).cloned()), args)
}

fn bilbo_with(var: Var, args: &[&str]) -> Run {
    if HELD.with(Cell::get) {
        return runner::bilbo_with(var, args, &tune);
    }
    let _shared = RUNS.read().unwrap_or_else(PoisonError::into_inner);
    runner::bilbo_with(var, args, &tune)
}

fn message(text: &str) -> String {
    json!({
        "model": "claude-test-1",
        "id": "msg_TEST0001",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 100, "output_tokens": 20}
    })
    .to_string()
}

fn response(text: &str) -> String {
    json!({
        "id": "resp_TEST0001",
        "object": "response",
        "status": "completed",
        "model": "gpt-test-1",
        "output": [{
            "type": "message",
            "id": "msg_TEST0001",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}]
        }],
        "usage": {"input_tokens": 100, "output_tokens": 20, "total_tokens": 120}
    })
    .to_string()
}

fn answer(text: &str) -> Reply {
    Reply::answer(200, &message(text))
}

const NOTE_TEXT: &str = "r1-7\nThe user asked about the garden fence.";

fn answers(count: usize) -> Vec<Reply> {
    (0..count).map(|_| answer(NOTE_TEXT)).collect()
}

fn config(url: &str, extra: &str) -> String {
    format!(
        "derive.enabled = true\nderive.provider = \"anthropic\"\nderive.in = [\"sessions\"]\nderive.quiet = \"0s\"\nderive.anthropic.model = \"claude-test-1\"\nderive.anthropic.api.endpoint = \"{url}/v1/messages\"\n{extra}"
    )
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = scratch::scratch_dir(test);
        for folder in ["home", "config", "sessions", "notes"] {
            fs::create_dir_all(dir.join(folder)).expect("create scratch");
        }
        let sandbox = Sandbox { dir };
        sandbox.configure("");
        sandbox
    }

    fn configure(&self, text: &str) {
        let mut whole = String::new();
        for provider in ["anthropic", "openai"] {
            if !text.contains(&format!("derive.{provider}.api.endpoint")) {
                whole.push_str(&format!("derive.{provider}.api.endpoint = \"{CLOSED}\"\n"));
            }
        }
        whole.push_str(text);
        fs::write(self.dir.join("config/config.toml"), whole).expect("a config file");
    }

    fn filled(test: &str, subagent: bool) -> Sandbox {
        Sandbox::filled_with(test, subagent, &[])
    }

    fn filled_with(test: &str, subagent: bool, extra: &[(&str, Vec<u8>)]) -> Sandbox {
        let sandbox = Sandbox::new(test);
        for (name, bytes) in extra {
            sandbox.write(name, bytes);
        }
        for (name, bytes) in SESSIONS {
            sandbox.write(&format!("sessions/{name}"), bytes);
        }
        if subagent {
            for (name, bytes) in SUBAGENT {
                sandbox.write(&format!("sessions/{name}"), bytes);
            }
        }
        for (name, bytes) in NOTES {
            sandbox.write(&format!("notes/{name}"), bytes);
        }
        sandbox.seal();
        sandbox
    }

    fn seal(&self) {
        let folders: Vec<&str> = ["sessions", "notes", "elsewhere"]
            .into_iter()
            .filter(|folder| self.dir.join(folder).exists())
            .collect();
        self.remember(&folders);
        let state = stores::layout(&self.vars()).state;
        stores::backdate(&state, 120);
        self.remember(&folders);
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
        vars.insert("ANTHROPIC_API_KEY".to_owned(), OsString::from(KEY));
        vars
    }

    fn run(&self, args: &[&str]) -> Run {
        bilbo(self.vars(), args)
    }

    fn run_asking(&self, args: &[&str]) -> (Run, Vec<bool>) {
        let vars = self.vars();
        let lock = self.derive_dir().join("lock");
        let asked = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&asked);
        let var: Var = Rc::new(move |name| {
            if name == "ANTHROPIC_API_KEY" {
                sink.borrow_mut().push(lock.exists());
            }
            vars.get(name).cloned()
        });
        let run = bilbo_with(var, args);
        let asked = asked.borrow().clone();
        (run, asked)
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

    fn store(&self) -> Store {
        let layout = stores::layout(&self.vars());
        Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("open the store")
    }

    fn sessions(&self) -> Vec<Id> {
        let store = self.store();
        let (changes, _) = store.changes(&Cursor::start()).expect("changes");
        let mut ids: Vec<Id> = changes
            .iter()
            .map(bilbo_store::entries::Change::id)
            .filter(|id| {
                store
                    .meta(*id)
                    .expect("meta")
                    .is_some_and(|meta| meta.kind == "session" && meta.labels.id.is_none())
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    fn session(&self, uuid: &str) -> Id {
        let store = self.store();
        self.sessions()
            .into_iter()
            .find(|id| {
                store
                    .meta(*id)
                    .expect("meta")
                    .and_then(|meta| meta.labels.capture)
                    .is_some_and(|label| label.as_str().contains(uuid))
            })
            .expect("a session")
    }

    fn notes_of(&self, parent: Id) -> Option<(Id, Generation, Vec<String>)> {
        let store = self.store();
        let label = CaptureLabel::new(format!("derive:{parent}")).expect("a label");
        let head = store.by_capture(&label).expect("by capture")?;
        if head.records == 0 {
            return None;
        }
        let records = store
            .read(
                head.id,
                head.generation,
                RecordRange {
                    first: 1,
                    count: head.records,
                },
            )
            .expect("records");
        Some((
            head.id,
            head.generation,
            records
                .iter()
                .map(|record| String::from_utf8_lossy(record).into_owned())
                .collect(),
        ))
    }

    fn of_kind(&self, kind: &str, belonging: bool) -> Vec<Id> {
        let store = self.store();
        let (changes, _) = store.changes(&Cursor::start()).expect("changes");
        let mut ids: Vec<Id> = changes
            .iter()
            .map(bilbo_store::entries::Change::id)
            .filter(|id| {
                store
                    .meta(*id)
                    .expect("meta")
                    .is_some_and(|meta| meta.kind == kind && meta.labels.id.is_some() == belonging)
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    fn derive_dir(&self) -> PathBuf {
        self.dir.join("state/bilbo/derive")
    }

    fn files(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut found = Vec::new();
        for folder in ["store", "state", "cache"] {
            let mut stack = vec![self.dir.join(folder)];
            while let Some(dir) = stack.pop() {
                let Ok(list) = fs::read_dir(&dir) else {
                    continue;
                };
                for entry in list {
                    let path = entry.expect("entry").path();
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        found.push((path.clone(), fs::read(&path).unwrap_or_default()));
                    }
                }
            }
        }
        found.sort();
        found
    }
}

fn requests(provider: &Provider) -> Vec<Request> {
    provider.requests()
}

fn body(request: &Request) -> String {
    String::from_utf8_lossy(&request.body).into_owned()
}

fn script(sandbox: &Sandbox, body: &str) -> String {
    let path = sandbox.dir.join("key.sh");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("a script");
    text(&path).to_owned()
}

fn text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 path")
}

#[test]
fn derive_is_off_until_its_three_keys_are_set() {
    let sandbox = Sandbox::filled("derive-off", false);
    let provider = Provider::start(Vec::new());
    let made = sandbox.dir.join("key-ran");
    let program = script(&sandbox, &format!("touch '{}'\necho {KEY}", made.display()));
    let command = format!("derive.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n");
    let complete = config(&provider.url(), &command);
    let cases = [
        (
            complete.replace("derive.enabled = true", "derive.enabled = false"),
            "bilbo: derive is off: derive.enabled is false",
        ),
        (
            complete.replace("derive.provider = \"anthropic\"", "derive.provider = \"\""),
            "bilbo: derive is off: derive.provider is not set",
        ),
        (
            complete.replace("derive.in = [\"sessions\"]", "derive.in = []"),
            "bilbo: derive is off: derive.in is empty",
        ),
    ];
    for (text, line) in cases {
        sandbox.configure(&text);
        let run = sandbox.run(&["derive"]);
        assert_eq!((run.code, run.stderr), (0, vec![line.to_owned()]), "{line}");
        assert_eq!(run.stdout, "");
    }
    assert!(requests(&provider).is_empty());
    assert!(!made.exists());
    assert!(!sandbox.derive_dir().exists());
}

#[test]
fn no_other_verb_sends_anything_with_derive_on() {
    let sandbox = Sandbox::filled("derive-others", false);
    let provider = Provider::start(Vec::new());
    sandbox.configure(&config(&provider.url(), ""));
    let id = sandbox.session(FIRST).to_string();
    for args in [
        vec!["remember", "sessions"],
        vec!["recall", "garden"],
        vec!["read", &id],
        vec!["reindex"],
        vec!["config"],
    ] {
        let args: Vec<String> = args
            .iter()
            .map(|arg| {
                if *arg == "sessions" {
                    sandbox.path("sessions")
                } else {
                    (*arg).to_owned()
                }
            })
            .collect();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let run = sandbox.run(&refs);
        assert_eq!(run.code, 0, "{refs:?}: {:?}", run.stderr);
    }
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_dry_run_says_what_a_run_would_send_and_sends_nothing() {
    let sandbox = Sandbox::filled("derive-dry", false);
    let provider = Provider::start(Vec::new());
    sandbox.configure(&config(&provider.url(), "").replace(
        "derive.in = [\"sessions\"]",
        "derive.in = [\"sessions\", \"nowhere\"]",
    ));
    let before = sandbox.files();
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, ["bilbo: derive.in 'nowhere' names no entry"]);
    let shipped = instruction::id(instruction::shipped()[0].1);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[0], format!("kinds: session (built in, {shipped})"));
    assert_eq!(
        lines[1],
        "named: 3 entries by derive.in, 0 of them left out by derive.not_in"
    );
    assert_eq!(lines[2], "due: 3 entries");
    assert_eq!(
        lines[3],
        "this run: 3 calls for 3 entries, 1007 tokens of records to send (derive.calls 20)"
    );
    assert_eq!(
        lines[4..],
        [
            format!(
                "provider: anthropic, via api, model claude-test-1, at {}/v1/messages",
                provider.url()
            ),
            "version: 2026-10-21".to_owned(),
            "key: the variable ANTHROPIC_API_KEY, which is set".to_owned(),
        ]
    );
    assert!(requests(&provider).is_empty());
    assert_eq!(sandbox.files(), before);
    assert!(!sandbox.derive_dir().exists());
    sandbox.configure(&config(&provider.url(), ""));
    let calling = Provider::start(answers(3));
    sandbox.configure(&config(&calling.url(), ""));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let log = fs::read_to_string(sandbox.derive_dir().join("log")).expect("a log");
    let sent: u64 = log
        .lines()
        .map(|line| {
            line.split('\t')
                .nth(10)
                .expect("a sent field")
                .parse::<u64>()
                .expect("a number")
        })
        .sum();
    assert_eq!(sent, 1007);
}

#[test]
fn a_dry_run_works_while_derive_is_off_and_names_each_key_unset() {
    let sandbox = Sandbox::filled("derive-dry-off", false);
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let shipped = instruction::id(instruction::shipped()[0].1);
    assert_eq!(
        run.stdout,
        format!(
            "off: derive.enabled is false; a run would send nothing\n\
             off: derive.provider is not set; a run would send nothing\n\
             off: derive.in is empty; a run would send nothing\n\
             kinds: session (built in, {shipped})\n\
             named: 0 entries by derive.in, 0 of them left out by derive.not_in\n\
             due: 0 entries\n\
             this run: 0 calls for 0 entries, 0 tokens of records to send (derive.calls 20)\n"
        )
    );
    sandbox.configure("derive.provider = \"gemini\"\n");
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        ["bilbo: derive.provider must be one of anthropic, openai, got \"gemini\""]
    );
}

#[test]
fn a_dry_run_reads_no_key_and_runs_no_program() {
    let sandbox = Sandbox::filled("derive-dry-key", false);
    let provider = Provider::start(Vec::new());
    let made = sandbox.dir.join("key-ran");
    let program = script(&sandbox, &format!("touch '{}'\necho {KEY}", made.display()));
    sandbox.configure(&config(
        &provider.url(),
        &format!("derive.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n"),
    ));
    let (run, asked) = sandbox.run_asking(&["derive", "--dry-run"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(!made.exists());
    assert_eq!(asked, Vec::<bool>::new());
    assert!(run.stdout.contains(&format!(
        "key: the program [\"/bin/sh\", \"{program}\"], not run\n"
    )));
    assert!(!run.stdout.contains(KEY));
}

#[test]
fn the_keys_variable_is_asked_once_and_only_when_a_call_is_made() {
    let _alone = alone();
    let key_command = |sandbox: &Sandbox| {
        let program = script(sandbox, &format!("echo {KEY}"));
        format!("derive.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n")
    };
    let sandbox = Sandbox::filled("derive-asks", false);
    let provider = Provider::start(answers(6));
    let off =
        config(&provider.url(), "").replace("derive.enabled = true", "derive.enabled = false");
    sandbox.configure(&off);
    let (run, asked) = sandbox.run_asking(&["derive"]);
    assert_eq!((run.code, asked.len()), (0, 0), "{:?}", run.stderr);

    sandbox.configure(&config(&provider.url(), ""));
    let (run, asked) = sandbox.run_asking(&["derive", "--dry-run"]);
    assert_eq!((run.code, asked.len()), (0, 1), "{:?}", run.stderr);

    sandbox.configure(&config(&provider.url(), &key_command(&sandbox)));
    let (run, asked) = sandbox.run_asking(&["derive", "--dry-run"]);
    assert_eq!((run.code, asked.len()), (0, 0), "{:?}", run.stderr);

    sandbox.configure(&config(&provider.url(), "").replace("\"0s\"", "\"24h\""));
    let (run, asked) = sandbox.run_asking(&["derive"]);
    assert_eq!((run.code, asked.len()), (0, 0), "{:?}", run.stderr);
    assert!(requests(&provider).is_empty());

    let calling = Sandbox::filled("derive-asks-call", false);
    calling.configure(&config(&provider.url(), ""));
    assert!(!calling.derive_dir().exists());
    let (run, asked) = calling.run_asking(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(asked, [true]);
    assert_eq!(requests(&provider).len(), 3);

    let other = Sandbox::filled("derive-asks-command", false);
    other.configure(&config(&provider.url(), &key_command(&other)));
    let (run, asked) = other.run_asking(&["derive"]);
    assert_eq!((run.code, asked.len()), (0, 0), "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 6);
}

fn long_session(id: &str, records: usize) -> Vec<u8> {
    let mut text = String::new();
    for at in 0..records {
        let words: Vec<String> = (0..260)
            .map(|word| format!("bed{}row{}", at * 7 + word % 11, word % 13))
            .collect();
        text.push_str(&format!(
            "{{\"parentUuid\":null,\"isSidechain\":false,\"type\":\"user\",\"uuid\":\"c1d2e3f4-{at:04}-4a5b-8c6d-7e8f9a0b1c2d\",\"timestamp\":\"2026-11-09T08:{at:02}:00.000Z\",\"userType\":\"external\",\"entrypoint\":\"cli\",\"cwd\":\"/home/me/src/garden\",\"sessionId\":\"{id}\",\"version\":\"2.1.300\",\"gitBranch\":\"main\",\"message\":{{\"role\":\"user\",\"content\":\"{}\"}}}}\n",
            words.join(" ")
        ));
    }
    text.into_bytes()
}

const LONG: &str = "3a7c9e1b-5d2f-4b8a-9c6e-1f0a2b3c4d5e";

#[test]
fn a_dry_run_with_refs_prints_a_line_a_ref() {
    let other = include_bytes!(
        "../../../libs/entries/rust/fixtures/session/7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl"
    )
    .to_vec();
    let sandbox = Sandbox::filled_with(
        "derive-refs",
        true,
        &[(
            "elsewhere/claude/q/7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl",
            other,
        )],
    );
    let provider = Provider::start(answers(2));
    let first = sandbox.session("8f2b6d4e");
    let second = sandbox.session("5c3a9e1f");
    let codex = sandbox.session("019a5b3c");
    let outside = sandbox.session("7d1e4b9a");
    let subagent = sandbox.of_kind("session", true)[0];
    let markdown = sandbox.of_kind("markdown", false)[0];
    sandbox.configure(&config(&provider.url(), "").replace(
        "derive.quiet = \"0s\"",
        &format!("derive.quiet = \"0s\"\nderive.not_in = [\"{codex}\"]"),
    ));
    let made = sandbox.run(&["derive", &second.to_string()]);
    assert_eq!(made.code, 0, "{:?}", made.stderr);
    let (note, _, records) = sandbox.notes_of(second).expect("notes");
    assert_eq!(records.len(), 2);
    let args: Vec<String> = [
        first, second, codex, subagent, outside, markdown, note, first,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    let mut argv = vec!["derive", "--dry-run"];
    argv.extend(args.iter().map(String::as_str));
    let run = sandbox.run(&argv);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let entries: Vec<&str> = run
        .stdout
        .lines()
        .filter(|l| l.starts_with("entry\t"))
        .collect();
    assert_eq!(
        entries,
        [
            format!("entry\t{first}\tdue\t-\t0"),
            format!("entry\t{second}\tcovered\t{note}\t1"),
            format!("entry\t{codex}\tleft out: derive.not_in\t-\t0"),
            format!("entry\t{subagent}\tleft out: belongs to another entry\t-\t0"),
            format!("entry\t{outside}\tleft out: not named by derive.in\t-\t0"),
            format!("entry\t{markdown}\tleft out: no instruction\t-\t0"),
            format!("entry\t{note}\tleft out: no instruction\t-\t0"),
        ]
    );
    assert!(requests(&provider).len() == 1);
}

#[test]
fn a_ref_with_a_locator_is_a_usage_error() {
    let sandbox = Sandbox::filled("derive-locator", false);
    let provider = Provider::start(Vec::new());
    sandbox.configure(&config(&provider.url(), ""));
    let id = sandbox.session(FIRST).to_string();
    for (given, bare) in [
        (format!("{id}#r3-9"), id.clone()),
        (format!("{id}#g1"), id.clone()),
    ] {
        for flag in [None, Some("--dry-run")] {
            let mut argv = vec!["derive"];
            argv.extend(flag);
            argv.push(&given);
            let run = sandbox.run(&argv);
            assert_eq!(run.code, 2, "{given}");
            assert_eq!(
                run.stderr[0],
                format!("bilbo: {given} names records; bilbo derive takes an entry: {bare}")
            );
            assert_eq!(run.stdout, "");
        }
    }
    let run = sandbox.run(&["derive", "zzz"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr[0].starts_with("bilbo: 'zzz' is not an entry id"),
        "{:?}",
        run.stderr
    );
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_ref_that_names_no_entry_is_said_as_read_says_it() {
    let sandbox = Sandbox::filled("derive-noentry", false);
    let provider = Provider::start(answers(1));
    sandbox.configure(&config(&provider.url(), ""));
    let good = sandbox.session(FIRST).to_string();
    let gone = Id::from_bytes([7; 16]).to_string();
    let run = sandbox.run(&["derive", &gone, "0000000001", "deadbeef", &good]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr[..3],
        [
            format!("bilbo: no entry {gone}"),
            "bilbo: no entry's id ends with 0000000001".to_owned(),
            "bilbo: no entry for the session deadbeef".to_owned(),
        ]
    );
    assert_eq!(requests(&provider).len(), 1);
}

#[test]
fn a_run_writes_notes_for_a_finished_entry() {
    let sandbox = Sandbox::filled("derive-run", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let run = sandbox.run(&["derive"]);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "entries 3, calls 3, notes 3, dropped 0, refused 0, failed 0"
    );
    assert!(
        lines[1].starts_with("tokens in 300, tokens out 60, seconds "),
        "{}",
        lines[1]
    );
    let parent = sandbox.session(FIRST);
    let (note, generation, records) = sandbox.notes_of(parent).expect("notes");
    assert_eq!(generation, Generation(1));
    assert_eq!(records.len(), 2);
    assert!(
        records[0].contains("The user asked about the garden fence."),
        "{records:?}"
    );
    let store = sandbox.store();
    let meta = store.meta(note).expect("meta").expect("an entry");
    assert_eq!(meta.kind, "note");
    assert_eq!(meta.labels.id, Some(parent));
    assert_eq!(
        meta.labels.capture.expect("a label").as_str(),
        format!("derive:{parent}")
    );
    drop(store);
    let found = sandbox.run(&["recall", "garden", "fence"]);
    assert_eq!(found.code, 0, "{:?}", found.stderr);
    let headers: Vec<&str> = found
        .stdout
        .lines()
        .filter(|l| l.starts_with("=== "))
        .collect();
    let short = |id: Id| id.short().to_string();
    let markdown = sandbox.of_kind("markdown", false)[0];
    assert_eq!(
        headers,
        [
            format!("=== {}#r1-21\t24 lines", short(markdown)),
            format!("=== {}#r3-20\t33 lines", short(parent)),
            format!("=== {}#r26-27\t4 lines\tpart 2/2", short(markdown)),
            format!("=== {}#r5-22\t30 lines", short(sandbox.session("019a5b3c"))),
            format!(
                "=== {}#r1\t2 lines\tnote",
                short(
                    sandbox
                        .notes_of(sandbox.session("019a5b3c"))
                        .expect("notes")
                        .0
                )
            ),
        ],
        "{}",
        found.stdout
    );
    assert!(
        found
            .stdout
            .contains("The user asked about the garden fence.")
    );
}

#[test]
fn a_long_entry_is_read_window_after_window_each_shown_the_notes_before() {
    let sandbox = Sandbox::filled_with(
        "derive-long",
        false,
        &[(
            "sessions/claude/p/3a7c9e1b-5d2f-4b8a-9c6e-1f0a2b3c4d5e.jsonl",
            long_session(LONG, 8),
        )],
    );
    let provider = Provider::start(vec![
        answer("r1-3\nThe user asked about the first beds."),
        answer("r4-6\nThe user asked about later beds."),
        answer("r7-8\nThe user asked about the last beds."),
    ]);
    sandbox.configure(&config(&provider.url(), "derive.window = 1000\n").replace(
        "derive.in = [\"sessions\"]",
        &format!("derive.in = [\"{}\"]", sandbox.session(LONG)),
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let sent = requests(&provider);
    assert_eq!(sent.len(), 3);
    let log = fs::read_to_string(sandbox.derive_dir().join("log")).expect("a log");
    assert_eq!(log.lines().count(), 3);
    let heading = |at: &str, text: &str| format!("{at} 2026-11-09\\n{text}");
    let first = heading("r1-3", "The user asked about the first beds.");
    let second = heading("r4-6", "The user asked about later beds.");
    assert!(!body(&sent[0]).contains("The user asked about the first beds."));
    assert!(body(&sent[1]).contains(&first), "{}", body(&sent[1]));
    assert!(body(&sent[2]).contains(&first), "{}", body(&sent[2]));
    assert!(body(&sent[2]).contains(&second), "{}", body(&sent[2]));
    let more = Provider::start(vec![answer("r9\nThe user asked about the new bed.")]);
    sandbox.configure(&config(&more.url(), "derive.window = 1000\n").replace(
        "derive.in = [\"sessions\"]",
        &format!("derive.in = [\"{}\"]", sandbox.session(LONG)),
    ));
    append(
        &sandbox,
        "sessions/claude/p/3a7c9e1b-5d2f-4b8a-9c6e-1f0a2b3c4d5e.jsonl",
        &user_line(LONG, 9, "Water the new bed."),
    );
    sandbox.seal();
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let sent = requests(&more);
    assert_eq!(sent.len(), 1);
    assert!(body(&sent[0]).contains(&first), "{}", body(&sent[0]));
    assert!(body(&sent[0]).contains(&second), "{}", body(&sent[0]));
}

fn user_line(session: &str, number: usize, text: &str) -> String {
    format!(
        "{{\"parentUuid\":null,\"isSidechain\":false,\"type\":\"user\",\"uuid\":\"d1e2f3a4-{number:04}-4a5b-8c6d-7e8f9a0b1c2d\",\"timestamp\":\"2026-11-09T09:{number:02}:00.000Z\",\"userType\":\"external\",\"entrypoint\":\"cli\",\"cwd\":\"/home/me/src/garden\",\"sessionId\":\"{session}\",\"version\":\"2.1.300\",\"gitBranch\":\"main\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n"
    )
}

fn append(sandbox: &Sandbox, name: &str, extra: &str) {
    let path = sandbox.dir.join(name);
    let mut bytes = fs::read(&path).expect("a session");
    bytes.extend_from_slice(extra.as_bytes());
    sandbox.write(name, &bytes);
}

const FIRST_FILE: &str = "sessions/claude/p/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl";

fn only(id: Id, url: &str) -> String {
    config(url, "").replace(
        "derive.in = [\"sessions\"]",
        &format!("derive.in = [\"{id}\"]"),
    )
}

#[test]
fn an_entry_that_grew_gets_notes_for_its_new_records_alone() {
    let sandbox = Sandbox::filled("derive-grew", false);
    let provider = Provider::start(vec![
        answer(NOTE_TEXT),
        answer("r24\nThe user asked about the tulips."),
    ]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 1);
    append(
        &sandbox,
        FIRST_FILE,
        &user_line(FIRST, 30, "Plant the tulips along the shed wall."),
    );
    sandbox.seal();
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let sent = requests(&provider);
    assert_eq!(sent.len(), 2);
    let second = body(&sent[1]);
    assert!(second.contains("tulips"), "{second}");
    assert!(!second.contains("carrot rows"), "{second}");
    assert!(body(&sent[0]).contains("carrot rows"));
    let (_, generation, records) = sandbox.notes_of(first).expect("notes");
    assert_eq!(generation, Generation(1));
    assert_eq!(records.len(), 4);
}

#[test]
fn a_rewritten_parent_is_derived_again_as_a_new_generation() {
    let sandbox = Sandbox::filled("derive-rewritten", false);
    let provider = Provider::start(answers(2));
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let rewritten = String::from_utf8(fs::read(sandbox.dir.join(FIRST_FILE)).expect("a session"))
        .expect("text")
        .replace("carrot", "parsnip");
    sandbox.write(FIRST_FILE, rewritten.as_bytes());
    sandbox.seal();
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 2);
    let (_, generation, records) = sandbox.notes_of(first).expect("notes");
    assert_eq!(generation, Generation(2));
    assert_eq!(records.len(), 2);
    assert!(body(&requests(&provider)[1]).contains("parsnip"));
}

#[test]
fn an_unnamed_entry_waits_for_quiet_and_a_named_one_does_not() {
    let sandbox = Sandbox::filled("derive-quiet", false);
    let provider = Provider::start(answers(3));
    let first = sandbox.session(FIRST);
    sandbox.configure(&config(&provider.url(), "").replace("\"0s\"", "\"24h\""));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(run.stdout.starts_with("entries 0, calls 0, notes 0,"));
    assert!(requests(&provider).is_empty());
    let named = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(named.code, 0, "{:?}", named.stderr);
    assert_eq!(requests(&provider).len(), 1);
    sandbox.configure(&config(&provider.url(), ""));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 3);
}

#[test]
fn a_named_entry_that_may_not_be_sent_is_said_and_the_others_are_sent() {
    let sandbox = Sandbox::filled_with(
        "derive-notsent",
        true,
        &[(
            "elsewhere/claude/q/7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl",
            include_bytes!(
                "../../../libs/entries/rust/fixtures/session/7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl"
            )
            .to_vec(),
        )],
    );
    let provider = Provider::start(answers(1));
    let codex = sandbox.session("019a5b3c");
    sandbox.configure(&config(&provider.url(), "").replace(
        "derive.quiet = \"0s\"",
        &format!("derive.quiet = \"0s\"\nderive.not_in = [\"{codex}\"]"),
    ));
    let markdown = sandbox.of_kind("markdown", false)[0];
    let subagent = sandbox.of_kind("session", true)[0];
    let outside = sandbox.session("7d1e4b9a");
    let good = sandbox.session(FIRST);
    let run = sandbox.run(&[
        "derive",
        &markdown.to_string(),
        &subagent.to_string(),
        &outside.to_string(),
        &codex.to_string(),
        &good.to_string(),
    ]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [
            format!("bilbo: {markdown} is of the kind markdown, which has no instruction"),
            format!("bilbo: {subagent} belongs to another entry"),
            format!("bilbo: {outside} is not named by derive.in"),
            format!("bilbo: {codex} is left out by derive.not_in"),
        ]
    );
    assert_eq!(requests(&provider).len(), 1);
}

#[test]
fn nothing_is_sent_while_the_index_is_behind_the_store() {
    let sandbox = Sandbox::filled("derive-behind", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let first = sandbox.session(FIRST);
    let layout = stores::layout(&sandbox.vars());
    notes::noted(&layout, first, &[b"{\"text\":\"a note\"}\n".to_vec()]);
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [
            "bilbo: the index is behind the store, so labels may be missing; bilbo remember brings it up to date"
        ]
    );
    assert!(requests(&provider).is_empty());
    sandbox.remember(&["sessions", "notes"]);
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 3);
}

struct Poisoned(Box<dyn bilbo_entries::reader::Reader>);

impl bilbo_entries::reader::Reader for Poisoned {
    fn kind(&self) -> &'static str {
        self.0.kind()
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<bilbo_entries::reader::Passage> {
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

    fn stands_alone(&self) -> bool {
        self.0.stands_alone()
    }
}

fn poisoned_remember(sandbox: &Sandbox, folder: &str) {
    let vars = sandbox.vars();
    let mut env = Env {
        var: std::rc::Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        notice: Box::new(|_| {}),
    };
    let settings = settings::load(
        &*env.var,
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .expect("settings");
    let readers: Vec<Box<dyn bilbo_entries::reader::Reader>> = bilbo_cli::readers(&settings)
        .expect("the readers")
        .into_iter()
        .map(|reader| Box::new(Poisoned(reader)) as Box<dyn bilbo_entries::reader::Reader>)
        .collect();
    let rendering = bilbo_cli::rendering(&settings).expect("the rendering");
    let disk = |cache: &Path| bilbo_cli::index(cache, &settings.search);
    let indexing = bilbo_cli::indexing(
        &settings,
        &readers,
        &rendering,
        &runner::shared_tokenizer,
        &disk,
    );
    let args = bilbo_cli::remember::Args {
        target: Some(PathBuf::from(sandbox.path(folder))),
        kind: None,
        follow: false,
    };
    bilbo_cli::remember::run(
        &bilbo_cli::bilbo(),
        args,
        &mut env,
        &settings,
        &bilbo_cli::kinds(&settings).expect("the kinds"),
        &indexing,
    )
    .expect("the remember");
}

#[test]
fn an_entry_whose_index_run_failed_is_not_sent() {
    let sandbox = Sandbox::filled("derive-failed-index", true);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let second = sandbox.session(SECOND);
    let subagent = sandbox.of_kind("session", true)[0];
    append(
        &sandbox,
        &format!("sessions/claude/p/{SECOND}.jsonl"),
        &user_line(SECOND, 40, "the POISON text"),
    );
    poisoned_remember(&sandbox, "sessions");
    let state = stores::layout(&sandbox.vars()).state;
    stores::backdate(&state, 120);
    poisoned_remember(&sandbox, "sessions");
    let run = sandbox.run(&[
        "derive",
        "--dry-run",
        &second.to_string(),
        &subagent.to_string(),
    ]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let lines: Vec<&str> = run
        .stdout
        .lines()
        .filter(|l| l.starts_with("entry\t"))
        .collect();
    assert_eq!(
        lines,
        [
            format!("entry\t{second}\tleft out: labels not current\t-\t0"),
            format!("entry\t{subagent}\tleft out: labels not current\t-\t0"),
        ]
    );
    let run = sandbox.run(&["derive", &second.to_string()]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: {second} has labels that are not current; bilbo remember brings the index up to date"
        )]
    );
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 2);
}

#[test]
fn the_calls_flag_beats_the_key() {
    let sandbox = Sandbox::filled("derive-calls", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), "derive.calls = 3\n"));
    let run = sandbox.run(&["derive", "--calls", "1"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 1);
    let sandbox = Sandbox::filled("derive-calls-dry", false);
    sandbox.configure(&config("http://127.0.0.1:9", "derive.calls = 3\n"));
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "2"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let this = dry
        .stdout
        .lines()
        .find(|line| line.starts_with("this run:"))
        .expect("a this run line");
    assert_eq!(
        this,
        "this run: 2 calls for 2 entries, 866 tokens of records to send (derive.calls 2)"
    );
    let run = sandbox.run(&["derive", "--calls", "0"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr[0],
        "bilbo: --calls must be a whole number from 1 to 100000, got \"0\""
    );
    assert_eq!(requests(&provider).len(), 1);
}

#[test]
fn redo_writes_again_what_another_instruction_wrote_and_then_finds_nothing() {
    let sandbox = Sandbox::filled("derive-redo", false);
    let provider = Provider::start(answers(6));
    sandbox.configure(&config(&provider.url(), ""));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    assert_eq!(requests(&provider).len(), 3);
    let file = sandbox.dir.join("instruction.txt");
    fs::write(&file, "Write notes, shortly.\n").expect("an instruction");
    sandbox.configure(&config(
        &provider.url(),
        &format!(
            "derive.instructions = {{ session = \"{}\" }}\n",
            file.display()
        ),
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(run.stdout.starts_with("entries 0, calls 0,"));
    assert_eq!(requests(&provider).len(), 3);
    let redo = sandbox.run(&["derive", "--redo"]);
    assert_eq!(redo.code, 0, "{:?}", redo.stderr);
    assert!(
        redo.stdout.starts_with("entries 3, calls 3, notes 3,"),
        "{}",
        redo.stdout
    );
    let first = sandbox.session(FIRST);
    assert_eq!(sandbox.notes_of(first).expect("notes").1, Generation(2));
    let again = sandbox.run(&["derive", "--redo"]);
    assert_eq!(again.code, 0, "{:?}", again.stderr);
    assert!(again.stdout.starts_with("entries 0, calls 0,"));
    assert_eq!(requests(&provider).len(), 6);

    let sandbox = Sandbox::filled("derive-redo-passed", false);
    let provider = Provider::start(vec![refusal("cyber"), answer(NOTE_TEXT)]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()).replace(
        "derive.quiet = \"0s\"",
        "derive.quiet = \"0s\"\nderive.tries = 1",
    ));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 1);
    let dry = sandbox.run(&["derive", "--dry-run", &first.to_string()]);
    let line = dry
        .stdout
        .lines()
        .find(|l| l.starts_with("entry\t"))
        .expect("a line");
    assert!(
        line.starts_with(&format!("entry\t{first}\tpassed over\t")),
        "{line}"
    );
    let dry = sandbox.run(&["derive", "--dry-run", "--redo", &first.to_string()]);
    let line = dry
        .stdout
        .lines()
        .find(|l| l.starts_with("entry\t"))
        .expect("a line");
    assert!(
        line.starts_with(&format!("entry\t{first}\tdue\t")),
        "{line}"
    );
    let redo = sandbox.run(&["derive", "--redo"]);
    assert_eq!(redo.code, 0, "{:?}", redo.stderr);
    assert!(
        redo.stdout.starts_with("entries 1, calls 1, notes 1,"),
        "{}",
        redo.stdout
    );
    assert_eq!(requests(&provider).len(), 2);
    assert!(body(&requests(&provider)[1]).contains("carrot rows"));
}

#[test]
fn redo_takes_another_way_to_the_same_model_for_the_same_writer() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-redo-way", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    assert_eq!(requests(&provider).len(), 3);
    sandbox.configure(&tool_config(&sandbox, &with_instruction()));
    let dry = sandbox.run(&["derive", "--redo", "--dry-run"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    assert!(
        dry.stdout.lines().any(|line| line == "due: 0 entries"),
        "{}",
        dry.stdout
    );
    let redo = sandbox.run(&["derive", "--redo"]);
    assert_eq!(redo.code, 0, "{:?}", redo.stderr);
    assert!(
        redo.stdout.starts_with("entries 0, calls 0,"),
        "{}",
        redo.stdout
    );
    assert!(!sandbox.dir.join("out/ran").exists());
    assert_eq!(requests(&provider).len(), 3);
}

fn refusal_body(category: &str) -> String {
    json!({
        "model": "claude-test-1",
        "content": [],
        "stop_reason": "refusal",
        "stop_details": {"type": "refusal", "category": category},
        "usage": {"input_tokens": 100, "output_tokens": 0}
    })
    .to_string()
}

fn refusal(category: &str) -> Reply {
    Reply::answer(200, &refusal_body(category))
}

fn cut() -> Reply {
    Reply::answer(
        200,
        &json!({
            "model": "claude-test-1",
            "content": [{"type": "thinking", "thinking": "", "signature": "c2ln"}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 100, "output_tokens": 16000}
        })
        .to_string(),
    )
}

fn failed_file(sandbox: &Sandbox) -> String {
    fs::read_to_string(sandbox.derive_dir().join("failed")).unwrap_or_default()
}

#[test]
fn a_refusal_is_said_charged_and_no_failure() {
    let sandbox = Sandbox::filled("derive-refusal", false);
    let provider = Provider::start(vec![refusal("cyber")]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some("entries 1, calls 1, notes 0, dropped 0, refused 1, failed 0")
    );
    assert_eq!(
        run.stderr,
        [format!("bilbo: refused {first} #r1-23: cyber")]
    );
    assert_eq!(failed_file(&sandbox).lines().count(), 1);
}

#[test]
fn a_failed_call_is_said_in_bilbos_words_and_exits_1() {
    let sandbox = Sandbox::filled("derive-cut", false);
    let provider = Provider::start(vec![cut()]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 1, notes 0, dropped 0, refused 0, failed 1\n")
    );
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: call failed for {first} #r1-23: the answer was cut at derive.answer_tokens; raise it or lower derive.window"
        )]
    );
}

fn error_body(kind: &str, text: &str) -> String {
    json!({"type": "error", "error": {"type": kind, "message": text}}).to_string()
}

fn overloaded() -> Reply {
    Reply::answer(529, &error_body("overloaded_error", "CANARY-MESSAGE busy"))
}

#[test]
fn a_window_refused_in_three_runs_is_passed_over_and_the_entry_goes_on() {
    let sandbox = Sandbox::filled("derive-passed", false);
    let provider = Provider::start(vec![refusal("cyber"), refusal("cyber"), refusal("cyber")]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    for _ in 0..3 {
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
    }
    assert_eq!(requests(&provider).len(), 3);
    let dry = sandbox.run(&["derive", "--dry-run", &first.to_string()]);
    let lines: Vec<&str> = dry.stdout.lines().collect();
    assert_eq!(lines[2], "due: 1 entries");
    assert_eq!(
        lines[3],
        "this run: 0 calls for 0 entries, 0 tokens of records to send (derive.calls 20)"
    );
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 3);
    assert!(run.stdout.starts_with("entries 0, calls 0, notes 0,"));
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: passed over {first} #r1-23: refused or failed in 3 runs"
        )]
    );
    let dry = sandbox.run(&["derive", "--dry-run", &first.to_string()]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let line = dry
        .stdout
        .lines()
        .find(|l| l.starts_with("entry\t"))
        .expect("a line");
    assert!(
        line.starts_with(&format!("entry\t{first}\tpassed over\t")),
        "{line}"
    );
    assert!(line.ends_with("\t0"), "{line}");
    let lines: Vec<&str> = dry.stdout.lines().collect();
    assert_eq!(lines[2], "due: 0 entries");
    assert_eq!(lines[3], "not due: 1 passed over");
}

#[test]
fn a_busy_provider_is_asked_again_after_the_wait_it_names() {
    let sandbox = Sandbox::filled("derive-busy-wait", false);
    let provider = Provider::start(vec![
        Reply::answer(529, &error_body("overloaded_error", "x")).with_header("retry-after", "1"),
        answer(NOTE_TEXT),
    ]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let sent = requests(&provider);
    assert_eq!(sent.len(), 2);
    assert!(sent[1].at.duration_since(sent[0].at) >= Duration::from_secs(1));
    assert!(
        run.stdout.starts_with("entries 1, calls 2, notes 1,"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_busy_provider_stops_the_run_after_its_retries() {
    let sandbox = Sandbox::filled("derive-busy-stop", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![overloaded()]);
    sandbox.configure(&only(first, &provider.url()).replace(
        "derive.quiet = \"0s\"",
        "derive.quiet = \"0s\"\nderive.retries = 0",
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [
            "bilbo: the provider is busy: it is overloaded (HTTP 529 overloaded_error); gave up after attempt 1"
        ]
    );
    let held = failed_file(&sandbox);
    assert!(
        held.lines().all(|line| line.starts_with("after\t")),
        "{held}"
    );

    let provider = Provider::start(vec![overloaded(), overloaded()]);
    sandbox.configure(&only(first, &provider.url()).replace(
        "derive.quiet = \"0s\"",
        "derive.quiet = \"0s\"\nderive.retries = 1\nderive.retry_wait = \"1s\"",
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [
            "bilbo: the provider is busy: it is overloaded (HTTP 529 overloaded_error); gave up after attempt 2"
        ]
    );
    assert_eq!(requests(&provider).len(), 2);

    let provider = Provider::start(vec![
        Reply::answer(429, &error_body("rate_limit_error", "x")).with_header("retry-after", "301"),
    ]);
    sandbox.configure(&only(first, &provider.url()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [
            "bilbo: the provider is busy: it limits the rate of requests (HTTP 429 rate_limit_error); the next attempt would wait \"301s\", more than derive.retry_limit"
        ]
    );
    assert_eq!(requests(&provider).len(), 1);
}

#[test]
fn a_stop_names_the_status_the_type_and_the_request_and_never_the_message() {
    let sandbox = Sandbox::filled("derive-stop", false);
    let provider = Provider::start(vec![
        Reply::answer(
            404,
            &error_body("not_found_error", "CANARY-MESSAGE the model is gone"),
        )
        .with_header("request-id", "req_test1"),
    ]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [
            "bilbo: the provider stopped the run: it has no such model or address (HTTP 404 not_found_error, request req_test1)"
        ]
    );
    assert!(!run.stdout.contains("CANARY"));
    for (path, bytes) in sandbox.files() {
        assert!(
            !bytes.windows(6).any(|window| window == b"CANARY"),
            "{}",
            path.display()
        );
    }
}

#[test]
fn three_failed_calls_in_a_row_stop_the_run_and_charge_nothing() {
    let sandbox = Sandbox::filled("derive-streak", false);
    let unreadable = || Reply::answer(200, "not an answer");
    let provider = Provider::start(vec![unreadable(), unreadable(), unreadable()]);
    sandbox.configure(&config(&provider.url(), ""));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some("bilbo: stopped: 3 calls failed in a row")
    );
    let held = failed_file(&sandbox);
    assert!(
        held.lines().all(|line| line.starts_with("after\t")),
        "{held}"
    );
    assert_eq!(requests(&provider).len(), 3);
}

#[test]
fn three_cut_answers_stop_the_run_and_charge_nothing() {
    let sandbox = Sandbox::filled("derive-cuts", false);
    let provider = Provider::start(vec![cut(), cut(), cut()]);
    sandbox.configure(&config(&provider.url(), "derive.failures = 3\n"));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some("entries 3, calls 3, notes 0, dropped 0, refused 0, failed 3")
    );
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some("bilbo: stopped: 3 calls failed in a row")
    );
    let held = failed_file(&sandbox);
    assert!(
        held.lines().all(|line| line.starts_with("after\t")),
        "{held}"
    );
    assert_eq!(requests(&provider).len(), 3);
}

#[test]
fn an_incomplete_provider_is_a_config_error_and_nothing_is_sent() {
    let sandbox = Sandbox::filled("derive-incomplete", false);
    let provider = Provider::start(Vec::new());
    let good = config(&provider.url(), "");
    let cases = [
        (
            good.replace(
                "derive.provider = \"anthropic\"",
                "derive.provider = \"gemini\"",
            ),
            "bilbo: derive.provider must be one of anthropic, openai, got \"gemini\"".to_owned(),
            KEY,
        ),
        (
            good.replace("derive.anthropic.model = \"claude-test-1\"\n", ""),
            "bilbo: derive.anthropic.model is not set".to_owned(),
            KEY,
        ),
    ];
    for (text, line, key) in cases {
        sandbox.configure(&text);
        let run = sandbox.run_with(&["derive"], &[("ANTHROPIC_API_KEY", key)]);
        assert_eq!((run.code, run.stderr), (2, vec![line.clone()]), "{line}");
    }
    sandbox.configure(&good.replace(
        "derive.in = [\"sessions\"]",
        "derive.in = [\"sessions\", \"nowhere\"]",
    ));
    let run = sandbox.run_with(&["derive"], &[("ANTHROPIC_API_KEY", "")]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        [
            "bilbo: derive.in 'nowhere' names no entry",
            "bilbo: ANTHROPIC_API_KEY is not set (derive.anthropic.api.key_env)"
        ]
    );
    let file = sandbox.path("config/config.toml");
    sandbox.configure(&good.replace(
        &format!("{}/v1/messages", provider.url()),
        "http://localhost:9/v1/messages",
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: {file}:7: derive.anthropic.api.endpoint must name the address in place of localhost: 127.0.0.1"
        )]
    );
    let password = "must hold no user name or password, got one";
    sandbox.configure(&good.replace(
        &format!("{}/v1/messages", provider.url()),
        "https://u:hunter2@example.com",
    ));
    let from_file = format!("bilbo: {file}:7: derive.anthropic.api.endpoint {password}");
    for argv in [&["derive"][..], &["config"]] {
        let run = sandbox.run(argv);
        assert_eq!((run.code, run.stderr.clone()), (2, vec![from_file.clone()]));
        assert!(!run.stdout.contains("hunter2"));
    }
    sandbox.configure(&config(&provider.url(), ""));
    let run = sandbox.run_with(
        &["derive"],
        &[
            ("ANTHROPIC_API_KEY", KEY),
            (
                "BILBO_DERIVE_ANTHROPIC_API_ENDPOINT",
                "https://u:hunter2@example.com",
            ),
        ],
    );
    let from_variable = format!("bilbo: BILBO_DERIVE_ANTHROPIC_API_ENDPOINT {password}");
    assert_eq!((run.code, run.stderr), (2, vec![from_variable.clone()]));
    assert!(!run.stdout.contains("hunter2"));
    let run = sandbox.run_with(
        &["config"],
        &[(
            "BILBO_DERIVE_ANTHROPIC_API_ENDPOINT",
            "https://u:hunter2@example.com",
        )],
    );
    assert_eq!((run.code, run.stderr), (2, vec![from_variable]));
    assert!(!run.stdout.contains("hunter2"));
    sandbox.configure(&good.replace("claude-test-1", KEY));
    let run = sandbox.run_with(&["derive"], &[("ANTHROPIC_API_KEY", KEY)]);
    assert_eq!(
        (run.code, run.stderr.clone()),
        (
            2,
            vec!["bilbo: derive.anthropic.model holds the key".to_owned()]
        )
    );
    assert!(!run.stdout.contains("PLUM"));
    assert!(!sandbox.derive_dir().join("log").exists());
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_bad_instruction_file_is_a_config_error() {
    let sandbox = Sandbox::filled("derive-instructions", false);
    let provider = Provider::start(Vec::new());
    let missing = sandbox.dir.join("missing.txt");
    let empty = sandbox.dir.join("empty.txt");
    let binary = sandbox.dir.join("binary.txt");
    let fine = sandbox.dir.join("fine.txt");
    fs::write(&empty, "  \n").expect("a file");
    fs::write(&binary, [0xff, 0xfe, 0xfd]).expect("a file");
    fs::write(&fine, "Write notes.\n").expect("a file");
    let cases = [
        (
            format!("session = \"{}\"", missing.display()),
            format!(
                "bilbo: derive.instructions.session: cannot read {}: No such file or directory (os error 2)",
                missing.display()
            ),
        ),
        (
            format!("session = \"{}\"", empty.display()),
            format!("bilbo: derive.instructions.session: {} is empty", empty.display()),
        ),
        (
            format!("session = \"{}\"", binary.display()),
            format!("bilbo: derive.instructions.session: {} is not UTF-8", binary.display()),
        ),
        (
            format!("note = \"{}\"", fine.display()),
            "bilbo: derive.instructions.note: no reader reads the kind 'note'; the kinds are markdown, session"
                .to_owned(),
        ),
    ];
    for (entry, line) in cases {
        sandbox.configure(&config(
            &provider.url(),
            &format!("derive.instructions = {{ {entry} }}\n"),
        ));
        let run = sandbox.run(&["derive"]);
        assert_eq!((run.code, run.stderr), (2, vec![line.clone()]), "{line}");
    }
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_second_derive_exits_0_while_one_holds_the_lock() {
    let sandbox = Sandbox::filled("derive-lock", false);
    let provider = Provider::start(answers(1));
    sandbox.configure(&config(&provider.url(), ""));
    fs::create_dir_all(sandbox.derive_dir()).expect("a folder");
    let held = fs::File::create(sandbox.derive_dir().join("lock")).expect("a lock file");
    held.set_permissions(fs::Permissions::from_mode(0o600))
        .expect("a mode");
    held.lock().expect("a lock");
    let run = sandbox.run(&["derive"]);
    assert_eq!(
        (run.code, run.stderr),
        (0, vec!["bilbo: another bilbo derive is running".to_owned()])
    );
    assert!(requests(&provider).is_empty());
    drop(held);
}

fn kept_block(sandbox: &Sandbox, parent: Id) -> Vec<u8> {
    let count = sandbox
        .store()
        .extent(parent, None)
        .expect("extent")
        .expect("an entry")
        .1;
    let paragraph = format!(
        "{{\"text\":\"The user asked about the garden fence.\",\"at\":1700000000,\"from\":[{{\"entry\":\"{parent}\",\"generation\":1,\"records\":[1,1]}}]}}"
    )
    .into_bytes();
    let run = bilbo_derive::record::Run {
        kind: "session".to_owned(),
        instruction: instruction::id(instruction::shipped()[0].1),
        provider: "anthropic".to_owned(),
        model: "claude-test-1".to_owned(),
        answered: None,
        same: true,
        made: 1_700_000_100,
        entry: parent,
        generation: Generation(1),
        first: 1,
        last: count,
        total: count,
        sent: 10,
        tokens_in: None,
        tokens_out: None,
        written: 1,
        kept: 1,
        range: 0,
        names: 0,
        passes: 0,
        passed: false,
    };
    bilbo_derive::spool::write(&bilbo_derive::spool::Block {
        parent,
        generation: Generation(1),
        generation_before: None,
        records_before: 0,
        opens: false,
        records: vec![paragraph, bilbo_derive::record::write(&run).into_bytes()],
    })
}

fn spooled(sandbox: &Sandbox, bytes: &[u8]) {
    let folder = sandbox.derive_dir().join("spool");
    fs::create_dir_all(&folder).expect("a folder");
    let block = folder.join("00000000000000000001.block");
    fs::write(&block, bytes).expect("a block");
    fs::set_permissions(&block, fs::Permissions::from_mode(0o600)).expect("a mode");
}

fn spool_names(sandbox: &Sandbox) -> Vec<String> {
    fs::read_dir(sandbox.derive_dir().join("spool"))
        .map(|list| {
            list.flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_run_that_died_after_a_call_keeps_its_notes_at_the_next_run() {
    let sandbox = Sandbox::filled("derive-died", false);
    let provider = Provider::start(Vec::new());
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    spooled(&sandbox, &kept_block(&sandbox, first));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let (_, generation, records) = sandbox.notes_of(first).expect("notes");
    assert_eq!((generation, records.len()), (Generation(1), 2));
    assert!(records[0].contains("The user asked about the garden fence."));
    assert!(spool_names(&sandbox).is_empty());
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_part_file_left_by_a_dead_run_is_removed() {
    let sandbox = Sandbox::filled("derive-part", false);
    let provider = Provider::start(Vec::new());
    sandbox.configure(&config(&provider.url(), "").replace("\"0s\"", "\"24h\""));
    let folder = sandbox.derive_dir().join("spool");
    fs::create_dir_all(&folder).expect("a folder");
    let first = sandbox.session(FIRST);
    spooled(&sandbox, &kept_block(&sandbox, first));
    fs::write(folder.join("00000000000000000007.part"), b"half a block").expect("a part");
    let run = sandbox.run(&["derive"]);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    assert!(spool_names(&sandbox).is_empty());
    let (_, _, records) = sandbox.notes_of(first).expect("notes");
    assert!(records[0].contains("The user asked about the garden fence."));
}

#[test]
fn a_config_that_sets_no_endpoint_points_both_apis_at_a_closed_port() {
    let sandbox = Sandbox::filled("derive-closed", false);
    sandbox.configure(
        "derive.enabled = true\nderive.provider = \"anthropic\"\nderive.in = [\"sessions\"]\nderive.anthropic.model = \"m\"\nderive.openai.model = \"m\"\n",
    );
    let line = |run: &Run| {
        run.stdout
            .lines()
            .find(|line| line.starts_with("provider: "))
            .map(str::to_owned)
    };
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(
        line(&run).as_deref(),
        Some("provider: anthropic, via api, model m, at http://127.0.0.1:9/")
    );
    sandbox.configure(
        "derive.enabled = true\nderive.provider = \"openai\"\nderive.in = [\"sessions\"]\nderive.anthropic.model = \"m\"\nderive.openai.model = \"m\"\n",
    );
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(
        line(&run).as_deref(),
        Some("provider: openai, via api, model m, at http://127.0.0.1:9/")
    );
}

#[test]
fn a_note_entry_cut_after_its_meta_frame_takes_its_block_at_the_next_run() {
    use bilbo_store::entries::{Capture, CaptureError, StoreError};
    use bilbo_store::segments::Step;
    let sandbox = Sandbox::filled("derive-meta", false);
    let provider = Provider::start(Vec::new());
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let layout = stores::layout(&sandbox.vars());
    let options = Options {
        hook: Some(Box::new(|step| {
            if step == Step::MetaSynced {
                Err(io::Error::other("stopped"))
            } else {
                Ok(())
            }
        })),
        ..Options::default()
    };
    let mut store = Store::open(&layout, options, &mut |_| {}).expect("open the store");
    let label = CaptureLabel::new(format!("derive:{first}")).expect("a label");
    let cut = store.capture(&Capture {
        label: &label,
        kind: bilbo_cli::notes().kind,
        id: None,
        id_label: Some(first),
        stamp: None,
        bytes: b"{\"text\":\"x\"}\n",
        ends: &[b"{\"text\":\"x\"}\n".len()],
    });
    assert!(matches!(
        cut,
        Err(CaptureError::Store(StoreError::Stopped(_)))
    ));
    drop(store);
    spooled(&sandbox, &kept_block(&sandbox, first));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let (_, _, records) = sandbox.notes_of(first).expect("notes");
    assert_eq!(records.len(), 2);
    assert!(spool_names(&sandbox).is_empty());
}

#[test]
fn remember_is_not_held_up_by_a_call() {
    let _together = together();
    let sandbox = Sandbox::filled("derive-unheld", false);
    let slow = Reply::trickle(&message(NOTE_TEXT), Duration::from_millis(20));
    let provider = Provider::start(vec![slow]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let vars = sandbox.vars();
    let calling = std::thread::spawn(move || {
        held_by_the_test();
        bilbo(vars, &["derive"])
    });
    let waited = std::time::Instant::now();
    while requests(&provider).is_empty() {
        assert!(
            waited.elapsed() < Duration::from_secs(20),
            "no request came"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let run = sandbox.run(&["remember", &sandbox.path("sessions")]);
    assert_eq!((run.code, run.stderr), (0, Vec::<String>::new()));
    assert!(!calling.is_finished(), "remember ended before the call did");
    let derived = calling.join().expect("the derive thread");
    assert_eq!(derived.code, 0, "{:?}", derived.stderr);
}

#[test]
fn the_log_gets_a_line_an_attempt_and_no_text_of_a_record() {
    let sandbox = Sandbox::filled("derive-log", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let log = sandbox.derive_dir().join("log");
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&log).expect("a log").permissions().mode() & 0o777
    };
    assert_eq!(mode, 0o600);
    let written = fs::read_to_string(&log).expect("a log");
    assert_eq!(written.lines().count(), 3);
    for line in written.lines() {
        assert_eq!(line.split('\t').count(), 20, "{line}");
        assert!(line.starts_with("d1\t"));
    }
    let mut folder = vec![sandbox.derive_dir()];
    while let Some(dir) = folder.pop() {
        for entry in fs::read_dir(dir).expect("a folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                folder.push(path);
            } else {
                let bytes = fs::read(&path).expect("a file");
                assert!(
                    !bytes.windows(6).any(|window| window == b"carrot"),
                    "{}",
                    path.display()
                );
            }
        }
    }
    assert!(!run.stdout.contains("carrot") && !run.stderr.join("\n").contains("carrot"));

    let other = Sandbox::filled("derive-log-open", false);
    let provider = Provider::start(answers(1));
    let first = other.session(FIRST);
    other.configure(&only(first, &provider.url()));
    fs::create_dir_all(other.derive_dir()).expect("a folder");
    let path = other.derive_dir().join("log");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&path, "").expect("a log");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("a mode");
    }
    let run = other.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: the calls were not logged: {} is not a private file of this user in a folder of this user",
            path.display()
        )]
    );
    assert!(other.notes_of(first).is_some());
}

fn every_record(sandbox: &Sandbox) -> Vec<Vec<u8>> {
    let store = sandbox.store();
    let (changes, _) = store.changes(&Cursor::start()).expect("changes");
    let mut ids: Vec<Id> = changes
        .iter()
        .map(bilbo_store::entries::Change::id)
        .collect();
    ids.sort();
    ids.dedup();
    let mut found = Vec::new();
    for id in ids {
        let Some((newest, _)) = store.extent(id, None).expect("extent") else {
            continue;
        };
        for generation in 1..=newest.0 {
            let Some((held, count)) = store
                .extent(id, Some(Generation(generation)))
                .expect("extent")
            else {
                continue;
            };
            if count > 0 {
                found.extend(
                    store
                        .read(id, held, RecordRange { first: 1, count })
                        .expect("records"),
                );
            }
        }
    }
    found
}

#[test]
fn the_key_is_in_nothing_the_run_wrote() {
    let sandbox = Sandbox::filled("derive-secret", false);
    let provider = Provider::start(vec![
        answer(NOTE_TEXT),
        refusal("cyber"),
        answer(&format!("r1-7\nThe key is {KEY} by mistake.")),
        Reply::answer(
            401,
            &error_body("authentication_error", "CANARY-MESSAGE bad key"),
        ),
    ]);
    sandbox.configure(&config(&provider.url(), ""));
    let one = sandbox.run(&["derive"]);
    let two = sandbox.run(&["derive"]);
    assert_eq!(two.code, 1);
    for run in [&one, &two] {
        assert!(!run.stdout.contains(KEY));
        assert!(!run.stderr.join("\n").contains(KEY));
    }
    for (path, bytes) in sandbox.files() {
        assert!(
            !bytes
                .windows(KEY.len())
                .any(|window| window == KEY.as_bytes()),
            "{}",
            path.display()
        );
    }
    for record in every_record(&sandbox) {
        assert!(
            !record
                .windows(KEY.len())
                .any(|window| window == KEY.as_bytes())
        );
    }
}

#[test]
fn two_stores_of_the_same_files_send_byte_equal_requests() {
    let mut all = Vec::new();
    for name in ["derive-equal-a", "derive-equal-b"] {
        let sandbox = Sandbox::filled(name, false);
        let provider = Provider::start(answers(3));
        sandbox.configure(&config(&provider.url(), ""));
        assert_eq!(sandbox.run(&["derive"]).code, 0);
        let mut bodies: Vec<String> = requests(&provider).iter().map(body).collect();
        bodies.sort();
        all.push(bodies);
    }
    assert_eq!(all[0].len(), 3);
    assert_eq!(all[0], all[1]);
}

fn tool_reply(result: &str) -> Value {
    json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "stop_reason": "end_turn",
        "num_turns": 1,
        "permission_denials": [],
        "result": result,
        "usage": {
            "input_tokens": 100,
            "output_tokens": 20,
            "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0}
        },
        "modelUsage": {"claude-test-1": {"webSearchRequests": 0}},
        "subagent_stats": {"spawned": 0}
    })
}

fn tool_config(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let script = sandbox.dir.join("tool.sh");
    let out = sandbox.dir.join("out");
    fs::create_dir_all(&out).expect("a folder");
    fs::write(
        sandbox.dir.join("reply.json"),
        tool_reply(NOTE_TEXT).to_string(),
    )
    .expect("a reply");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho ran >> {out}/ran\nprintf '%s\\n' \"$@\" > {out}/args\npwd > {out}/pwd\ncat > /dev/null\ncat {dir}/reply.json\n",
            out = out.display(),
            dir = sandbox.dir.display()
        ),
    )
    .expect("a script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
    let items: Vec<String> = arguments.iter().map(|item| format!("{item:?}")).collect();
    format!(
        "derive.enabled = true\nderive.provider = \"anthropic\"\nderive.in = [\"sessions\"]\nderive.quiet = \"0s\"\nderive.anthropic.via = \"tool\"\nderive.anthropic.model = \"claude-test-1\"\n\n[derive.anthropic.tool]\nprogram = {:?}\nproject_files = [\"zz-marker.md\"]\narguments = [{}]\n",
        script.display().to_string(),
        items.join(", ")
    )
}

const TOOL_ARGUMENTS: [&str; 11] = [
    "-p",
    "--model",
    "{model}",
    "--tools",
    "",
    "--strict-mcp-config",
    "--safe-mode",
    "--no-session-persistence",
    "--effort",
    "{effort}",
    "--system-prompt",
];

fn with_instruction() -> Vec<&'static str> {
    let mut items: Vec<&'static str> = TOOL_ARGUMENTS.to_vec();
    items.push("{instruction}");
    items
}

#[test]
fn a_run_through_the_tool_writes_notes_and_reads_no_key() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-tool", false);
    sandbox.configure(&tool_config(&sandbox, &with_instruction()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 3, calls 3, notes 3, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    let first = sandbox.session(FIRST);
    let (_, generation, records) = sandbox.notes_of(first).expect("notes");
    assert_eq!(generation, Generation(1));
    assert!(records[0].contains("The user asked about the garden fence."));
    let work = stores::layout(&sandbox.vars()).state.join("derive/work");
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&work).expect("a folder").permissions().mode() & 0o777
    };
    assert_eq!(mode, 0o700);
    let args = fs::read_to_string(sandbox.dir.join("out/args")).expect("args");
    assert!(args.lines().any(|line| line == "claude-test-1"), "{args}");
    let ran_in = fs::read_to_string(sandbox.dir.join("out/pwd")).expect("pwd");
    assert_eq!(
        fs::canonicalize(ran_in.trim()).expect("the folder it ran in"),
        fs::canonicalize(&work).expect("the work folder")
    );
}

#[test]
fn the_tools_arguments_are_what_the_settings_say() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-tool-args", false);
    let mut items = with_instruction();
    items.insert(0, "--fallback-model");
    items.insert(1, "zz-value");
    sandbox.configure(&tool_config(&sandbox, &items));
    let first = sandbox.session(FIRST).to_string();
    let run = sandbox.run(&["derive", &first]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let args = fs::read_to_string(sandbox.dir.join("out/args")).expect("args");
    assert_eq!(args.lines().next(), Some("--fallback-model"), "{args}");
    assert_eq!(args.lines().nth(1), Some("zz-value"), "{args}");
    fs::remove_file(sandbox.dir.join("out/ran")).expect("ran");

    let without: Vec<&str> = with_instruction()
        .into_iter()
        .filter(|item| *item != "--strict-mcp-config")
        .collect();
    sandbox.configure(&tool_config(&sandbox, &without));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        [
            "bilbo: derive.anthropic.tool.arguments must keep the model without tools: it needs \"--strict-mcp-config\""
        ]
    );
    assert!(!sandbox.dir.join("out/ran").exists());
}

fn capture_session(sandbox: &Sandbox, id: Id, id_label: Option<Id>, label: &str, text: &[u8]) {
    let layout = stores::layout(&sandbox.vars());
    let mut ends = Vec::new();
    for (at, byte) in text.iter().enumerate() {
        if *byte == b'\n' {
            ends.push(at + 1);
        }
    }
    let label = CaptureLabel::new(label.to_owned()).expect("a capture label");
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open the store");
    store
        .capture(&Capture {
            label: &label,
            kind: "session",
            id: Some(id),
            id_label,
            stamp: None,
            bytes: text,
            ends: &ends,
        })
        .expect("capture");
    store.close().expect("close");
}

fn twin(front: u8) -> Id {
    let mut bytes = [9u8; 16];
    bytes[0] = front;
    Id::from_bytes(bytes)
}

#[test]
fn an_ambiguous_not_in_value_leaves_out_every_entry_it_ends() {
    let sandbox = Sandbox::filled("derive-ambiguous", false);
    let one = twin(1);
    let two = twin(2);
    capture_session(&sandbox, one, None, "twin-one", SESSIONS[0].1);
    capture_session(&sandbox, two, None, "twin-two", SESSIONS[1].1);
    sandbox.seal();
    let provider = Provider::start(answers(3));
    let short = one.short().to_string();
    assert!(two.short().to_string() == short);
    sandbox.configure(
        &config(&provider.url(), &format!("derive.not_in = [\"{short}\"]\n")).replace(
            "derive.in = [\"sessions\"]",
            &format!("derive.in = [\"sessions\", \"{one}\", \"{two}\"]"),
        ),
    );
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: derive.not_in '{short}' ends the id of 2 entries: {one}, {two}; all of them are left out"
        )]
    );
    assert_eq!(
        run.stdout.lines().next(),
        Some("entries 3, calls 3, notes 3, dropped 0, refused 0, failed 0")
    );
    assert_eq!(requests(&provider).len(), 3);
    for left in [one, two] {
        let dry = sandbox.run(&["derive", "--dry-run", &left.to_string()]);
        assert!(
            dry.stdout
                .contains(&format!("entry\t{left}\tleft out: derive.not_in\t-\t0")),
            "{}",
            dry.stdout
        );
    }
}

#[test]
fn a_keys_program_runs_in_the_state_folder() {
    let _alone = alone();
    for provider_name in ["anthropic", "openai"] {
        let (test, replies) = match provider_name {
            "anthropic" => ("derive-key-folder", answers(3)),
            _ => (
                "derive-key-folder-openai",
                (0..3)
                    .map(|_| Reply::answer(200, &response(NOTE_TEXT)))
                    .collect(),
            ),
        };
        let sandbox = Sandbox::filled(test, false);
        let provider = Provider::start(replies);
        let wrote = sandbox.dir.join("where");
        let program = script(
            &sandbox,
            &format!("pwd > '{}'\necho {KEY}", wrote.display()),
        );
        let key =
            format!("derive.{provider_name}.api.key_command = [\"/bin/sh\", \"{program}\"]\n");
        sandbox.configure(&match provider_name {
            "anthropic" => config(&provider.url(), &key),
            _ => format!(
                "derive.enabled = true\nderive.provider = \"openai\"\nderive.in = [\"sessions\"]\nderive.quiet = \"0s\"\nderive.openai.model = \"gpt-test-1\"\nderive.openai.api.endpoint = \"{}/v1/responses\"\n{key}",
                provider.url()
            ),
        });
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{provider_name}: {:?}", run.stderr);
        assert_eq!(requests(&provider).len(), 3, "{provider_name}");
        let state = stores::layout(&sandbox.vars()).state;
        assert_eq!(
            fs::canonicalize(fs::read_to_string(&wrote).expect("a folder").trim()).expect("a path"),
            fs::canonicalize(state).expect("a path"),
            "{provider_name}"
        );
    }
}

fn refs_run(sandbox: &Sandbox, args: &[&str], gone: &[String]) -> (Run, Vec<String>) {
    let mut argv = vec!["derive"];
    argv.extend(args);
    argv.extend(gone.iter().map(String::as_str));
    (sandbox.run(&argv), Vec::new())
}

#[test]
fn a_run_with_only_refs_that_name_nothing_sends_nothing() {
    let sandbox = Sandbox::filled("derive-refs-nothing", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let gone = Id::from_bytes([7; 16]).to_string();
    let lines = [
        format!("bilbo: no entry {gone}"),
        "bilbo: no entry for the session deadbeef".to_owned(),
    ];
    for (refs, said) in [
        (vec![gone.clone()], &lines[..1]),
        (vec!["deadbeef".to_owned()], &lines[1..]),
        (vec![gone.clone(), "deadbeef".to_owned()], &lines[..]),
    ] {
        let (run, _) = refs_run(&sandbox, &[], &refs);
        assert_eq!(run.code, 1, "{refs:?}");
        assert_eq!(run.stderr, said, "{refs:?}");
        let totals: Vec<&str> = run.stdout.lines().collect();
        assert_eq!(
            totals[0],
            "entries 0, calls 0, notes 0, dropped 0, refused 0, failed 0"
        );
        assert!(totals[1].starts_with("tokens in 0, tokens out 0, seconds "));
    }
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_dry_run_with_only_refs_that_name_nothing_counts_nothing() {
    let sandbox = Sandbox::filled("derive-refs-dry", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let gone = Id::from_bytes([7; 16]).to_string();
    for refs in [
        vec![gone.clone()],
        vec!["deadbeef".to_owned()],
        vec![gone, "deadbeef".to_owned()],
    ] {
        let (run, _) = refs_run(&sandbox, &["--dry-run"], &refs);
        assert_eq!(run.code, 1, "{refs:?}");
        let lines: Vec<&str> = run.stdout.lines().collect();
        assert_eq!(
            lines[1..4],
            [
                "named: 0 entries by derive.in, 0 of them left out by derive.not_in",
                "due: 0 entries",
                "this run: 0 calls for 0 entries, 0 tokens of records to send (derive.calls 20)",
            ],
            "{refs:?}"
        );
        assert!(!lines.iter().any(|line| line.starts_with("entry\t")));
    }
    assert!(requests(&provider).is_empty());
}

#[test]
fn nothing_is_sent_with_no_index() {
    let sandbox = Sandbox::filled("derive-no-index", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let cache = stores::layout(&sandbox.vars()).cache;
    fs::remove_dir_all(cache.join("index")).expect("an index folder");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        ["bilbo: there is no search index, so labels cannot be read; bilbo remember makes one"]
    );
    assert_eq!(run.stdout, "");
    assert!(requests(&provider).is_empty());
}

#[test]
fn nothing_is_sent_with_an_index_another_bilbo_made() {
    let sandbox = Sandbox::filled("derive-other-index", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), "markdown.heading_width = 90\n"));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        [
            "bilbo: the search index was made by another bilbo, so labels cannot be read; bilbo remember makes it again"
        ]
    );
    assert!(requests(&provider).is_empty());
}

#[test]
fn an_index_that_fails_after_the_capture_keeps_the_notes_and_exits_1() {
    let sandbox = Sandbox::filled("derive-index-fails", false);
    let provider = Provider::start(answers(1));
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    REFUSING.with(|flag| flag.set(true));
    let run = sandbox.run(&["derive"]);
    REFUSING.with(|flag| flag.set(false));
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(
        run.stdout.starts_with("entries 1, calls 1, notes 1,"),
        "{:?} {}",
        run.stderr,
        run.stdout
    );
    assert_eq!(
        run.stderr,
        [
            "bilbo: the search index was not updated: cannot commit the index /refused: refused; bilbo reindex makes the index again"
        ]
    );
    assert_eq!(requests(&provider).len(), 1);
    let (_, _, records) = sandbox.notes_of(first).expect("notes");
    assert!(records[0].contains("The user asked about the garden fence."));
}

fn only_ids(url: &str, ids: &[Id], extra: &str) -> String {
    let list: Vec<String> = ids.iter().map(|id| format!("\"{id}\"")).collect();
    config(url, extra).replace(
        "derive.in = [\"sessions\"]",
        &format!("derive.in = [{}]", list.join(", ")),
    )
}

#[test]
fn a_dry_run_counts_each_state_in_its_own_field() {
    let sandbox = Sandbox::filled("derive-states", false);
    let provider = Provider::start(vec![answer(NOTE_TEXT), refusal("cyber"), refusal("cyber")]);
    let first = sandbox.session(FIRST);
    let second = sandbox.session(SECOND);
    let [covered, passed, left_out, belongs, growing] =
        [21u8, 22, 23, 24, 25].map(|n| Id::from_bytes([n; 16]));
    for (id, label, under) in [
        (covered, "covered", None),
        (passed, "passed", None),
        (left_out, "left-out", None),
        (belongs, "belongs", Some(first)),
    ] {
        capture_session(&sandbox, id, under, label, SESSIONS[0].1);
    }
    sandbox.seal();
    let all = [first, second, covered, passed, left_out, growing, belongs];
    let extra = format!("derive.tries = 1\nderive.not_in = [\"{left_out}\"]\n");
    sandbox.configure(&only_ids(&provider.url(), &all, &extra));
    for (id, code) in [(covered, 0), (passed, 0), (passed, 0)] {
        let run = sandbox.run(&["derive", &id.to_string()]);
        assert_eq!(run.code, code, "{:?}", run.stderr);
    }
    assert_eq!(requests(&provider).len(), 2);
    append(
        &sandbox,
        &format!("sessions/claude/p/{SECOND}.jsonl"),
        &user_line(SECOND, 40, "the POISON text"),
    );
    capture_session(&sandbox, growing, None, "growing", SESSIONS[0].1);
    poisoned_remember(&sandbox, "sessions");
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[1..4],
        [
            "named: 5 entries by derive.in, 1 of them left out by derive.not_in",
            "due: 1 entries",
            "not due: 1 still growing, 1 covered, 1 belong to another entry, 1 passed over, 1 with labels not current",
        ],
        "{}",
        run.stdout
    );
}

#[test]
fn an_index_that_cannot_be_read_is_an_error_and_not_a_missing_index() {
    let sandbox = Sandbox::filled("derive-unreadable-index", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    let cache = stores::layout(&sandbox.vars()).cache;
    let folder = cache.join("index");
    let meta = folder.join("meta.json");
    fs::set_permissions(&meta, fs::Permissions::from_mode(0o000)).expect("a mode");
    let recall = sandbox.run(&["recall", "garden"]);
    let run = sandbox.run(&["derive"]);
    fs::set_permissions(&meta, fs::Permissions::from_mode(0o600)).expect("a mode");
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.stderr.len(), 1);
    assert_eq!(run.stderr, recall.stderr);
    assert!(!run.stderr[0].contains("no search index"));
    assert_eq!(run.stdout, "");
    assert!(requests(&provider).is_empty());
}

fn state_folder(sandbox: &Sandbox) -> PathBuf {
    sandbox.dir.join("state/bilbo")
}

#[test]
fn a_state_folder_others_can_write_is_refused_before_the_lock() {
    let sandbox = Sandbox::filled("derive-open-state", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), ""));
    fs::set_permissions(state_folder(&sandbox), fs::Permissions::from_mode(0o777)).expect("a mode");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: {} can be written by others; bilbo will not use it",
            state_folder(&sandbox).display()
        )]
    );
    assert_eq!(run.stdout, "");
    assert!(!sandbox.derive_dir().exists());
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_spool_that_cannot_be_opened_stops_the_run_before_any_call() {
    let sandbox = Sandbox::filled("derive-unlistable", false);
    let provider = Provider::start(answers(3));
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let block = kept_block(&sandbox, first);
    spooled(&sandbox, &block);
    let spool = sandbox.derive_dir().join("spool");
    fs::set_permissions(&spool, fs::Permissions::from_mode(0o300)).expect("a mode");
    let run = sandbox.run(&["derive"]);
    fs::set_permissions(&spool, fs::Permissions::from_mode(0o700)).expect("a mode");
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: cannot list {}: Permission denied (os error 13)",
            spool.display()
        )]
    );
    assert!(requests(&provider).is_empty());
    let kept = spool.join("00000000000000000001.block");
    assert_eq!(fs::read(&kept).expect("the block"), block);
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(sandbox.notes_of(first).is_some());
}

#[test]
fn a_state_file_that_is_no_private_file_of_this_user_stops_the_run() {
    let sandbox = Sandbox::filled("derive-state-files", false);
    let provider = Provider::start(answers(3));
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let derive = sandbox.derive_dir();
    let spool = derive.join("spool");
    let refused = |path: &Path| {
        format!(
            "bilbo: {} is not a private file of this user in a folder of this user",
            path.display()
        )
    };
    fs::create_dir_all(&spool).expect("folders");
    for folder in [&derive, &spool] {
        fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).expect("a mode");
    }
    let failed = derive.join("failed");
    fs::write(&failed, "").expect("charges");
    fs::set_permissions(&failed, fs::Permissions::from_mode(0o600)).expect("a mode");
    symlink(&failed, derive.join("lock")).expect("a link");
    let run = sandbox.run(&["derive"]);
    assert_eq!(
        (run.code, run.stderr),
        (1, vec![refused(&derive.join("lock"))])
    );
    fs::remove_file(derive.join("lock")).expect("unlinked");

    let block = spool.join("00000000000000000001.block");
    spooled(&sandbox, &kept_block(&sandbox, first));
    fs::hard_link(&block, spool.join("00000000000000000002.block")).expect("a second name");
    let run = sandbox.run(&["derive"]);
    assert_eq!((run.code, run.stderr), (1, vec![refused(&block)]));
    assert!(sandbox.notes_of(first).is_none());
    fs::remove_file(spool.join("00000000000000000002.block")).expect("removed");
    fs::remove_file(&block).expect("removed");

    fs::remove_file(&failed).expect("removed");
    let outside = sandbox.dir.join("outside");
    fs::write(&outside, "").expect("a file");
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).expect("a mode");
    symlink(&outside, &failed).expect("a link");
    let run = sandbox.run(&["derive"]);
    assert_eq!((run.code, run.stderr), (1, vec![refused(&failed)]));
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_log_that_is_a_fifo_is_refused_and_the_run_ends() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-fifo-log", false);
    let provider = Provider::start(answers(1));
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let derive = sandbox.derive_dir();
    fs::create_dir_all(derive.join("spool")).expect("folders");
    for folder in [derive.clone(), derive.join("spool")] {
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("a mode");
    }
    let log = derive.join("log");
    let made = std::process::Command::new("mkfifo")
        .arg(&log)
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    let vars = sandbox.vars();
    let running = std::thread::spawn(move || {
        held_by_the_test();
        bilbo(vars, &["derive"])
    });
    let waited = std::time::Instant::now();
    while !running.is_finished() {
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "the run did not end"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let run = running.join().expect("the run");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: the calls were not logged: {} is not a private file of this user in a folder of this user",
            log.display()
        )]
    );
    assert!(sandbox.notes_of(first).is_some());
}

#[test]
fn a_file_of_charges_that_cannot_be_read_stops_the_run() {
    let sandbox = Sandbox::filled("derive-unreadable-charges", false);
    let provider = Provider::start(answers(3));
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let derive = sandbox.derive_dir();
    fs::create_dir_all(derive.join("spool")).expect("folders");
    for folder in [derive.clone(), derive.join("spool")] {
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("a mode");
    }
    let failed = derive.join("failed");
    let extent = Charged {
        parent: first,
        generation: Generation(1),
        first: 1,
        last: 23,
    };
    let text = format!("{0}\n{0}\n{0}\n", bilbo_derive::tries::line(&extent, 3));
    fs::write(&failed, &text).expect("charges");
    fs::set_permissions(&failed, fs::Permissions::from_mode(0o000)).expect("a mode");
    for args in [vec!["derive"], vec!["derive", "--dry-run"]] {
        let run = sandbox.run(&args);
        assert_eq!(run.code, 1, "{args:?} {:?}", run.stderr);
        assert_eq!(
            run.stderr,
            [format!(
                "bilbo: cannot read {}: Permission denied (os error 13)",
                failed.display()
            )]
        );
    }
    fs::set_permissions(&failed, fs::Permissions::from_mode(0o600)).expect("a mode");
    assert_eq!(fs::read_to_string(&failed).expect("charges"), text);
    assert!(requests(&provider).is_empty());
}

#[test]
fn a_line_of_charges_no_window_holds_goes_at_the_next_write() {
    let sandbox = Sandbox::filled("derive-stale-charges", false);
    let provider = Provider::start(vec![refusal("cyber"), refusal("cyber")]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(failed_file(&sandbox).lines().count(), 1);
    append(
        &sandbox,
        FIRST_FILE,
        &user_line(FIRST, 30, "Plant the tulips along the shed wall."),
    );
    sandbox.seal();
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let charges = failed_file(&sandbox);
    assert_eq!(charges.lines().count(), 1, "{charges}");
    assert!(
        charges.lines().all(|line| line.ends_with("\t1")),
        "{charges}"
    );
}

fn waiting_for(provider: &Provider, count: usize) {
    let waited = std::time::Instant::now();
    while requests(provider).len() < count {
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "no request came"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_error_during_the_calls_comes_after_what_the_run_collected() {
    let _together = together();
    let sandbox = Sandbox::filled("derive-error-calls", false);
    let slow = Reply::trickle(&refusal_body("cyber"), Duration::from_millis(15));
    let provider = Provider::start(vec![slow]);
    let first = sandbox.session(FIRST);
    sandbox.configure(
        &only(first, &provider.url()).replace("derive.in = [\"", "derive.in = [\"nowhere\", \""),
    );
    let vars = sandbox.vars();
    let running = std::thread::spawn(move || {
        held_by_the_test();
        bilbo(vars, &["derive"])
    });
    waiting_for(&provider, 1);
    let derive = sandbox.derive_dir();
    fs::set_permissions(&derive, fs::Permissions::from_mode(0o500)).expect("a mode");
    let run = running.join().expect("the run");
    fs::set_permissions(&derive, fs::Permissions::from_mode(0o700)).expect("a mode");
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [
            "bilbo: derive.in 'nowhere' names no entry".to_owned(),
            format!(
                "bilbo: cannot write {}: Permission denied (os error 13)",
                derive.join("failed.tmp").display()
            ),
        ]
    );
    assert_eq!(run.stdout, "");
}

#[test]
fn an_error_after_the_calls_prints_what_the_run_said_and_its_counts() {
    let _together = together();
    let sandbox = Sandbox::filled("derive-error-after", false);
    let slow = Reply::trickle(&message(NOTE_TEXT), Duration::from_millis(15));
    let provider = Provider::start(vec![refusal("cyber"), slow, answer(NOTE_TEXT)]);
    let first = sandbox.session(FIRST);
    let second = sandbox.session(SECOND);
    sandbox.configure(&only_ids(&provider.url(), &[first, second], ""));
    let vars = sandbox.vars();
    let running = std::thread::spawn(move || {
        held_by_the_test();
        bilbo(vars, &["derive"])
    });
    waiting_for(&provider, 2);
    let state = state_folder(&sandbox);
    fs::set_permissions(&state, fs::Permissions::from_mode(0o500)).expect("a mode");
    let run = running.join().expect("the run");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).expect("a mode");
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.stderr.len(), 2, "{:?}", run.stderr);
    assert!(
        run.stderr[0].starts_with("bilbo: refused ") && run.stderr[0].ends_with(" #r1-23: cyber"),
        "{:?}",
        run.stderr
    );
    assert_eq!(
        run.stderr[1],
        format!(
            "bilbo: cannot create {}: Permission denied (os error 13)",
            state.join("open.since").display()
        )
    );
    let totals: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        totals[0],
        "entries 2, calls 2, notes 1, dropped 0, refused 1, failed 0"
    );
    let again = sandbox.run(&["derive"]);
    assert_eq!(again.code, 0, "{:?}", again.stderr);
    assert_eq!(requests(&provider).len(), 3);
}

#[test]
fn two_runs_in_a_row_begin_at_different_entries_when_the_first_proved_nothing() {
    let sandbox = Sandbox::filled("derive-turned", false);
    let provider = Provider::start(vec![Reply::answer(200, "not an answer"), answer(NOTE_TEXT)]);
    let first = sandbox.session(FIRST);
    let second = sandbox.session(SECOND);
    sandbox.configure(&only_ids(&provider.url(), &[first, second], ""));
    let one = sandbox.run(&["derive", "--calls", "1"]);
    assert_eq!(one.code, 1, "{:?}", one.stderr);
    assert!(
        failed_file(&sandbox)
            .lines()
            .any(|line| line.starts_with("after\t")),
        "{}",
        failed_file(&sandbox)
    );
    let two = sandbox.run(&["derive", "--calls", "1"]);
    assert_eq!(two.code, 0, "{:?}", two.stderr);
    let sent = requests(&provider);
    assert_eq!(sent.len(), 2);
    assert_ne!(body(&sent[0]), body(&sent[1]));
    assert!(
        !failed_file(&sandbox)
            .lines()
            .any(|line| line.starts_with("after\t"))
    );
}

fn holds(dir: &Path, needle: &str) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            holds(&path, needle)
        } else {
            fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
            })
        }
    })
}

#[test]
fn a_run_of_passes_only_runs_no_key_program_and_writes_the_model_nowhere() {
    let sandbox = Sandbox::filled("derive-passes-only", false);
    let provider = Provider::start(vec![refusal("cyber"), refusal("cyber"), refusal("cyber")]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only(first, &provider.url()));
    for _ in 0..3 {
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
    }
    let made = sandbox.dir.join("key-ran");
    let program = script(&sandbox, &format!("touch '{}'\necho {KEY}", made.display()));
    let extra = format!("derive.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n");
    let settled = config(&provider.url(), &extra).replace(
        "derive.in = [\"sessions\"]",
        &format!("derive.in = [\"{first}\"]"),
    );
    let at = settled.find("derive.anthropic.model").expect("a model");
    let end = at + settled[at..].find('\n').expect("a line end");
    let settled = format!(
        "{}derive.anthropic.model = \"{KEY}\"{}",
        &settled[..at],
        &settled[end..]
    );
    sandbox.configure(&settled);
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 3);
    assert!(!made.exists());
    let vars = sandbox.vars();
    let layout = stores::layout(&vars);
    assert!(!holds(&layout.state, KEY));
    assert!(!holds(&layout.root, KEY));
}

#[test]
fn the_run_says_why_the_tool_was_not_started() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-tool-not-started", false);
    let config = tool_config(&sandbox, &with_instruction());
    let missing = sandbox.dir.join("no-such-tool");
    let config = config.replace(
        &format!(
            "program = {:?}",
            sandbox.dir.join("tool.sh").display().to_string()
        ),
        &format!("program = {:?}", missing.display().to_string()),
    );
    sandbox.configure(&config);
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some(
            format!(
                "bilbo: the provider stopped the run: its program could not be started: {}: No such file or directory (os error 2)",
                missing.display()
            )
            .as_str()
        )
    );
    assert!(!sandbox.dir.join("out/ran").exists());

    sandbox.configure(&tool_config(&sandbox, &with_instruction()));
    let work = stores::layout(&sandbox.vars()).state.join("derive/work");
    fs::create_dir_all(work.join("left")).expect("a folder");
    fs::write(work.join("left/file"), "x").expect("a file");
    fs::set_permissions(&work, fs::Permissions::from_mode(0o700)).expect("a mode");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert!(
        run.stderr
            .last()
            .is_some_and(|line| line.ends_with(" holds a folder that is not empty: left")),
        "{:?}",
        run.stderr
    );
}

#[test]
fn the_run_names_the_field_a_reply_lacks() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-tool-field", false);
    sandbox.configure(&tool_config(&sandbox, &with_instruction()));
    let mut reply = tool_reply(NOTE_TEXT);
    reply
        .as_object_mut()
        .expect("an object")
        .remove("subagent_stats");
    fs::write(sandbox.dir.join("reply.json"), reply.to_string()).expect("a reply");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some(
            "bilbo: the reply holds no \"subagent_stats\": bilbo reads the reply as claude 2.1.327 prints it"
        )
    );
    assert!(
        run.stderr
            .iter()
            .any(|line| line.contains("calls failed in a row"))
    );
    let held = failed_file(&sandbox);
    assert!(
        held.lines().all(|line| line.starts_with("after\t")),
        "{held}"
    );
}
