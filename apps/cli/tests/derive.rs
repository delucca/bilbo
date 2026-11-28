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

use bilbo_cli::console::env::{Env, Var, Who};
use bilbo_cli::console::indexing::Indexing;
use bilbo_cli::console::settings;
use bilbo_derive::instruction;
use bilbo_derive::tries::Extent as Charged;
use bilbo_entries::tag::{self, Change};
use bilbo_provider::{Provider, Reply, Request};
use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats};
use bilbo_search::position::Position;
use bilbo_store::entries::{Capture, Cursor, Entries, Generation, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::labels::{By, CaptureLabel, Standing, Tag, Tags};
use bilbo_store::segments::{Options, Store};
use serde_json::{Value, json};

mod notes;
mod runner;
mod sandbox;
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
        "derive.enabled = true\nllm.provider = \"anthropic\"\nderive.in = [\"sessions\"]\nderive.quiet = \"0s\"\nllm.anthropic.model = \"claude-test-1\"\nllm.anthropic.api.endpoint = \"{url}/v1/messages\"\n{extra}"
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
        self.write_config(text, true);
    }

    fn as_given(&self, text: &str) {
        self.write_config(text, false);
    }

    fn write_config(&self, text: &str, closed_summary: bool) {
        let file = self.dir.join("config/config.toml");
        fs::write(&file, text).expect("a config file");
        let read = self.run(&["config"]);
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
                whole.push_str(&format!("llm.{provider}.api.endpoint = \"{CLOSED}\"\n"));
            }
        }
        whole.push_str(text);
        if closed_summary && !text.contains("derive.summary") {
            whole.push_str("\n[derive.summary]\nenabled = false\n");
        }
        fs::write(file, whole).expect("a config file");
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
        sandbox::write(&path, bytes);
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
    let command = format!("llm.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n");
    let complete = config(&provider.url(), &command);
    let cases = [
        (
            complete.replace("derive.enabled = true", "derive.enabled = false"),
            "bilbo: derive is off: derive.enabled is false",
        ),
        (
            complete.replace("llm.provider = \"anthropic\"", "llm.provider = \"\""),
            "bilbo: derive is off: llm.provider is not set",
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
        vec!["forget", &id],
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
fn the_request_holds_the_answer_length_the_settings_give() {
    let sandbox = Sandbox::filled("derive-answer-length", false);
    let provider = Provider::start(answers(3));
    sandbox.configure(&config(&provider.url(), "derive.answer_tokens = 1234\n"));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let first = &requests(&provider)[0];
    let body: Value = serde_json::from_slice(&first.body).expect("a JSON body");
    assert_eq!(body["max_tokens"], json!(1234), "{body}");
}

#[test]
fn the_response_bound_of_the_settings_reaches_a_call() {
    let sandbox = Sandbox::filled("derive-response-bound", false);
    let long = format!("{NOTE_TEXT}{}", " plain words".repeat(200));
    let provider = Provider::start(vec![answer(&long)]);
    let first = sandbox.session(FIRST);
    sandbox.configure(&only_ids(
        &provider.url(),
        &[first],
        "llm.response = \"1 KiB\"\n",
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 1, notes 0, dropped 0, refused 0, failed 1\n")
    );
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: call failed for {first} #r1-23: what came back is over llm.response"
        )]
    );
}

#[test]
fn a_sandbox_with_no_endpoint_of_its_own_closes_both_providers() {
    let sandbox = Sandbox::new("derive-closed");
    let file = sandbox.path("config/config.toml");
    let endpoints = |sandbox: &Sandbox| -> Vec<String> {
        let run = sandbox.run(&["config"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        run.stdout
            .lines()
            .filter(|line| line.contains(".api.endpoint = "))
            .map(str::to_owned)
            .collect()
    };
    let expected = [
        format!("llm.anthropic.api.endpoint = \"{CLOSED}\" ({file}:1)"),
        format!("llm.openai.api.endpoint = \"{CLOSED}\" ({file}:2)"),
    ];
    assert_eq!(endpoints(&sandbox), expected, "the constructor's config");
    for text in [
        "# llm.anthropic.api.endpoint and llm.openai.api.endpoint are set nowhere\n",
        "[llm.openai.api.extra]\nllm.anthropic.api.endpoint = \"unused\"\n",
    ] {
        sandbox.configure(text);
        assert_eq!(endpoints(&sandbox), expected, "{text:?}");
    }
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
             off: llm.provider is not set; a run would send nothing\n\
             off: derive.in is empty; a run would send nothing\n\
             kinds: session (built in, {shipped})\n\
             named: 0 entries by derive.in, 0 of them left out by derive.not_in\n\
             due: 0 entries\n\
             this run: 0 calls for 0 entries, 0 tokens of records to send (derive.calls 20)\n"
        )
    );
    sandbox.configure("llm.provider = \"gemini\"\n");
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        ["bilbo: llm.provider must be one of anthropic, openai, got \"gemini\""]
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
        &format!("llm.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n"),
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
        format!("llm.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n")
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
            "bilbo: the index is behind the store, so labels may be missing; bilbo refresh brings it up to date"
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
        who: Who::Nobody,
        program: None,
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
        tags: Vec::new(),
        follow: false,
        edit: false,
    };
    let note = bilbo_cli::notes();
    let written = bilbo_cli::remember::note::Written {
        kind: note.kind,
        record: note.record,
        label: note.label,
    };
    bilbo_cli::remember::run(
        &bilbo_cli::bilbo(),
        args,
        &mut env,
        &settings,
        &bilbo_cli::kinds(&settings).expect("the kinds"),
        &indexing,
        &bilbo_cli::remember::Handed {
            written: &written,
            labelled: &bilbo_cli::labelled,
        },
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
            "bilbo: {second} has labels that are not current; bilbo refresh brings the index up to date"
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
            good.replace("llm.provider = \"anthropic\"", "llm.provider = \"gemini\""),
            "bilbo: llm.provider must be one of anthropic, openai, got \"gemini\"".to_owned(),
            KEY,
        ),
        (
            good.replace("llm.anthropic.model = \"claude-test-1\"\n", ""),
            "bilbo: llm.anthropic.model is not set".to_owned(),
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
            "bilbo: ANTHROPIC_API_KEY is not set (llm.anthropic.api.key_env)"
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
            "bilbo: {file}:7: llm.anthropic.api.endpoint must name the address in place of localhost: 127.0.0.1"
        )]
    );
    let password = "must hold no user name or password, got one";
    sandbox.configure(&good.replace(
        &format!("{}/v1/messages", provider.url()),
        "https://u:hunter2@example.com",
    ));
    let from_file = format!("bilbo: {file}:7: llm.anthropic.api.endpoint {password}");
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
                "BILBO_LLM_ANTHROPIC_API_ENDPOINT",
                "https://u:hunter2@example.com",
            ),
        ],
    );
    let from_variable = format!("bilbo: BILBO_LLM_ANTHROPIC_API_ENDPOINT {password}");
    assert_eq!((run.code, run.stderr), (2, vec![from_variable.clone()]));
    assert!(!run.stdout.contains("hunter2"));
    let run = sandbox.run_with(
        &["config"],
        &[(
            "BILBO_LLM_ANTHROPIC_API_ENDPOINT",
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
            vec!["bilbo: llm.anthropic.model holds the key".to_owned()]
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
        "derive.enabled = true\nllm.provider = \"anthropic\"\nderive.in = [\"sessions\"]\nllm.anthropic.model = \"m\"\nllm.openai.model = \"m\"\n",
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
        "derive.enabled = true\nllm.provider = \"openai\"\nderive.in = [\"sessions\"]\nllm.anthropic.model = \"m\"\nllm.openai.model = \"m\"\n",
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
        tags: &Tags::default(),
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
        "derive.enabled = true\nllm.provider = \"anthropic\"\nderive.in = [\"sessions\"]\nderive.quiet = \"0s\"\nllm.anthropic.via = \"tool\"\nllm.anthropic.model = \"claude-test-1\"\n\n[llm.anthropic.tool]\nprogram = {:?}\nproject_files = [\"zz-marker.md\"]\narguments = [{}]\n",
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
            "bilbo: llm.anthropic.tool.arguments must keep the model without tools: it needs \"--strict-mcp-config\""
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
            tags: &Tags::default(),
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
        let key = format!("llm.{provider_name}.api.key_command = [\"/bin/sh\", \"{program}\"]\n");
        sandbox.configure(&match provider_name {
            "anthropic" => config(&provider.url(), &key),
            _ => format!(
                "derive.enabled = true\nllm.provider = \"openai\"\nderive.in = [\"sessions\"]\nderive.quiet = \"0s\"\nllm.openai.model = \"gpt-test-1\"\nllm.openai.api.endpoint = \"{}/v1/responses\"\n{key}",
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
        ["bilbo: there is no search index, so labels cannot be read; bilbo refresh makes one"]
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
            "bilbo: the search index was made by another bilbo, so labels cannot be read; bilbo refresh makes it again"
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
    let extra = format!("llm.anthropic.api.key_command = [\"/bin/sh\", \"{program}\"]\n");
    let settled = config(&provider.url(), &extra).replace(
        "derive.in = [\"sessions\"]",
        &format!("derive.in = [\"{first}\"]"),
    );
    let at = settled.find("llm.anthropic.model").expect("a model");
    let end = at + settled[at..].find('\n').expect("a line end");
    let settled = format!(
        "{}llm.anthropic.model = \"{KEY}\"{}",
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

fn codex_reply(text: &str) -> String {
    [
        json!({"type": "thread.started", "thread_id": "00000000-0000-7000-8000-000000000001"}),
        json!({"type": "turn.started"}),
        json!({"type": "item.completed", "item": {"id": "item_0", "type": "agent_message", "text": text}}),
        json!({"type": "turn.completed", "usage": {
            "input_tokens": 1200,
            "cached_input_tokens": 0,
            "cache_write_input_tokens": 0,
            "output_tokens": 80,
            "reasoning_output_tokens": 30
        }}),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect()
}

fn codex_failure(message: &str) -> String {
    [
        json!({"type": "thread.started", "thread_id": "00000000-0000-7000-8000-000000000001"}),
        json!({"type": "turn.started"}),
        json!({"type": "error", "message": message}),
        json!({"type": "turn.failed", "error": {"message": message}}),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect()
}

fn codex_home(sandbox: &Sandbox) -> PathBuf {
    sandbox.derive_dir().join("codex")
}

fn codex_config(sandbox: &Sandbox, reply: &str) -> String {
    let script = sandbox.dir.join("tool.sh");
    let out = sandbox.dir.join("out");
    fs::create_dir_all(&out).expect("a folder");
    fs::write(sandbox.dir.join("reply.jsonl"), reply).expect("a reply");
    let models = json!({"models": [{
        "slug": "model-a",
        "display_name": "Model A",
        "supported_reasoning_levels": [
            {"effort": "low", "description": "a"},
            {"effort": "medium", "description": "b"},
            {"effort": "high", "description": "c"}
        ],
        "tool_mode": "code_mode_only",
        "apply_patch_tool_type": "freeform",
        "experimental_supported_tools": ["clock"],
        "supports_search_tool": true
    }]});
    fs::write(sandbox.dir.join("models.json"), models.to_string()).expect("a catalog");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$1\" >> {out}/calls\ncase \"$1\" in\n  --version) echo 'codex-cli 0.169.0' ;;\n  debug) cat {dir}/models.json ;;\n  exec) echo ran >> {out}/ran\n        n=$(wc -l < {out}/ran | tr -d ' ')\n        printf '%s\\n' \"$@\" > {out}/args; pwd > {out}/pwd; env > {out}/env\n        cat > /dev/null\n        r={dir}/reply.jsonl; if [ -f {dir}/reply-$n.jsonl ]; then r={dir}/reply-$n.jsonl; fi\n        c=0; if [ -f {dir}/exit-$n ]; then c=$(cat {dir}/exit-$n); fi\n        cat $r; exit $c ;;\nesac\n",
            out = out.display(),
            dir = sandbox.dir.display()
        ),
    )
    .expect("a script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("a mode");
    let home = codex_home(sandbox);
    codex_login(&home);
    format!(
        "derive.enabled = true\nllm.provider = \"openai\"\nderive.in = [\"sessions\"]\nderive.quiet = \"0s\"\nllm.openai.via = \"tool\"\nllm.openai.model = \"model-a\"\n\n[llm.openai.tool]\nprogram = {:?}\nsystem_files = []\n",
        script.display().to_string()
    )
}

fn codex_login(home: &Path) {
    fs::create_dir_all(home).expect("a home");
    fs::set_permissions(home, fs::Permissions::from_mode(0o700)).expect("a mode");
    fs::write(home.join("auth.json"), "").expect("a login");
}

fn logged_server_error(sandbox: &Sandbox) -> bool {
    fs::read_to_string(sandbox.derive_dir().join("log"))
        .expect("a log")
        .lines()
        .any(|line| line.contains("server-error"))
}

fn ran(sandbox: &Sandbox) -> usize {
    fs::read_to_string(sandbox.dir.join("out/ran")).map_or(0, |text| text.lines().count())
}

#[test]
fn a_run_through_codex_writes_notes_and_reads_no_key() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-codex", false);
    sandbox.configure(&codex_config(&sandbox, &codex_reply(NOTE_TEXT)));
    let run = sandbox.run_with(&["derive"], &[("OPENAI_API_KEY", KEY)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 3, calls 3, notes 3, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    let first = sandbox.session(FIRST);
    let (_, _, records) = sandbox.notes_of(first).expect("notes");
    assert!(records[0].contains("The user asked about the garden fence."));
    let work = sandbox.derive_dir().join("work");
    let ran_in = fs::read_to_string(sandbox.dir.join("out/pwd")).expect("pwd");
    assert_eq!(
        fs::canonicalize(ran_in.trim()).expect("the folder it ran in"),
        fs::canonicalize(&work).expect("the work folder")
    );
    let env = fs::read_to_string(sandbox.dir.join("out/env")).expect("env");
    assert!(
        env.lines()
            .any(|line| line == format!("CODEX_HOME={}", codex_home(&sandbox).display())),
        "{env}"
    );
    assert!(
        !env.lines().any(|line| line.starts_with("OPENAI_API_KEY=")),
        "{env}"
    );
    let args = fs::read_to_string(sandbox.dir.join("out/args")).expect("args");
    assert!(args.lines().any(|line| line == "model-a"), "{args}");
    assert_eq!(ran(&sandbox), 3);
}

#[test]
fn the_run_says_why_codex_was_not_started() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-codex-not-started", false);
    sandbox.configure(&codex_config(&sandbox, &codex_reply(NOTE_TEXT)));
    let home = codex_home(&sandbox);
    fs::remove_dir_all(&home).expect("no home");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some(
            format!(
                "bilbo: llm.openai.tool.home: {0} does not exist: make it, then log in with CODEX_HOME={0} codex login",
                home.display()
            )
            .as_str()
        )
    );
    assert_eq!(ran(&sandbox), 0);
    assert!(!sandbox.dir.join("out/calls").exists());
    codex_login(&home);
    fs::write(home.join("AGENTS.md"), "x").expect("a setup file");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert!(
        run.stderr.last().is_some_and(|line| line.ends_with(&format!(
            "llm.openai.tool.home: {}/AGENTS.md is the user's setup for codex, and bilbo's codex home holds none",
            home.display()
        ))),
        "{:?}",
        run.stderr
    );
    assert_eq!(ran(&sandbox), 0);
    assert!(!sandbox.dir.join("out/calls").exists());
}

#[test]
fn the_dry_run_of_codex_names_its_home_and_starts_nothing() {
    let sandbox = Sandbox::filled("derive-codex-dry", false);
    sandbox.configure(&codex_config(&sandbox, &codex_reply(NOTE_TEXT)));
    let home = codex_home(&sandbox);
    fs::remove_dir_all(&home).expect("no home");
    let run = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let script = sandbox.dir.join("tool.sh");
    assert!(
        run.stdout.lines().any(|line| line
            == format!(
                "provider: openai, via tool, model model-a, effort medium, program {}",
                script.display()
            )),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .lines()
            .any(|line| line == format!("home: {}", home.display())),
        "{}",
        run.stdout
    );
    assert_eq!(ran(&sandbox), 0);
    assert!(!sandbox.dir.join("out/calls").exists());
    assert!(!home.exists());
}

#[test]
fn a_lost_login_stops_the_run_in_the_tools_words() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-codex-login", false);
    sandbox.configure(&codex_config(
        &sandbox,
        &codex_failure("unexpected status 401 Unauthorized: x"),
    ));
    fs::write(sandbox.dir.join("exit-1"), "1").expect("an exit");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(
        run.stderr.iter().any(|line| line
            == "bilbo: the provider stopped the run: the tool is not logged in, or its login is no longer taken (exit 1 401)"),
        "{:?}",
        run.stderr
    );
    assert!(
        run.stderr.iter().any(|line| line.ends_with(&format!(
            "log in again with CODEX_HOME={} codex login",
            codex_home(&sandbox).display()
        ))),
        "{:?}",
        run.stderr
    );
    assert_eq!(ran(&sandbox), 1);
    assert!(
        run.stderr
            .iter()
            .all(|line| !line.contains("Unauthorized: x")),
        "{:?}",
        run.stderr
    );
    for (path, bytes) in sandbox.files() {
        assert!(
            !bytes
                .windows(b"Unauthorized: x".len())
                .any(|window| window == b"Unauthorized: x"),
            "{}",
            path.display()
        );
    }
}

#[test]
fn an_error_codex_does_not_explain_ends_the_run_and_charges_no_window() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-codex-unknown", false);
    let config = codex_config(&sandbox, &codex_reply(NOTE_TEXT)).replace(
        "derive.quiet = \"0s\"\n",
        "derive.quiet = \"0s\"\nderive.tries = 1\n",
    );
    sandbox.configure(&config);
    fs::write(
        sandbox.dir.join("reply-2.jsonl"),
        codex_failure("SENTINEL-7f3a not a sentence bilbo knows"),
    )
    .expect("a failed turn");
    fs::write(sandbox.dir.join("exit-2"), "1").expect("an exit");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some("bilbo: the provider stopped the run: it had an error of its own (exit 1)")
    );
    assert_eq!(ran(&sandbox), 2);
    assert!(!run.stdout.contains("SENTINEL-7f3a"));
    assert!(
        run.stderr
            .iter()
            .all(|line| !line.contains("SENTINEL-7f3a"))
    );
    for (path, bytes) in sandbox.files() {
        assert!(
            !bytes
                .windows(b"SENTINEL-7f3a".len())
                .any(|window| window == b"SENTINEL-7f3a"),
            "{}",
            path.display()
        );
    }
    assert!(logged_server_error(&sandbox));
    fs::remove_file(sandbox.dir.join("reply-2.jsonl")).expect("a reply");
    fs::remove_file(sandbox.dir.join("exit-2")).expect("an exit");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 2, calls 2, notes 2, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    assert_eq!(ran(&sandbox), 4);
}

#[test]
fn a_tool_codex_reached_ends_the_run_and_charges_no_window() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-codex-reached", false);
    let config = codex_config(&sandbox, &codex_reply(NOTE_TEXT)).replace(
        "derive.quiet = \"0s\"\n",
        "derive.quiet = \"0s\"\nderive.tries = 1\n",
    );
    sandbox.configure(&config);
    let reached = codex_reply(NOTE_TEXT).replacen(
        "{\"item\"",
        "{\"item\":{\"id\":\"item_0\",\"type\":\"command_execution\"},\"type\":\"item.started\"}\n{\"item\"",
        1,
    );
    fs::write(sandbox.dir.join("reply-2.jsonl"), reached).expect("a reply");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    let stop = run
        .stderr
        .iter()
        .position(|line| {
            line == "bilbo: the provider stopped the run: it had an error of its own (exit 0)"
        })
        .unwrap_or_else(|| panic!("{:?}", run.stderr));
    assert!(
        run.stderr[stop + 1..]
            .last()
            .is_some_and(|line| line.ends_with(
                "codex reached a tool of its own: bilbo's arguments did not keep it without tools"
            )),
        "{:?}",
        run.stderr
    );
    assert_eq!(ran(&sandbox), 2);
    let kept = sandbox
        .sessions()
        .into_iter()
        .filter(|id| sandbox.notes_of(*id).is_some())
        .count();
    assert_eq!(kept, 1);
    assert!(logged_server_error(&sandbox));
    fs::remove_file(sandbox.dir.join("reply-2.jsonl")).expect("a reply");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 2, calls 2, notes 2, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
}

fn declare_through_the_store(sandbox: &Sandbox, id: Id, add: &[&str]) {
    let add: std::collections::BTreeSet<Tag> = add
        .iter()
        .map(|name| Tag::new(name).expect("a tag"))
        .collect();
    let none = std::collections::BTreeSet::new();
    let layout = stores::layout(&sandbox.vars());
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open the store");
    tag::declare(
        id,
        &Change::Person {
            add: &add,
            remove: &none,
        },
        32,
        &mut store,
    )
    .expect("declare");
}

#[test]
fn a_tag_change_neither_stops_derive_nor_puts_the_index_behind() {
    let sandbox = Sandbox::filled("derive-tag-change", false);
    let provider = Provider::start(Vec::new());
    sandbox.configure(&config(&provider.url(), ""));
    declare_through_the_store(&sandbox, sandbox.session(FIRST), &["garden"]);

    let run = sandbox.run(&["derive", "--dry-run"]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, Vec::<String>::new());
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[2], "due: 3 entries");
    assert!(!run.stdout.contains("labels not current"), "{}", run.stdout);
}

#[test]
fn derive_in_takes_a_tag_and_derive_not_in_leaves_a_tagged_entry_out() {
    let sandbox = Sandbox::filled("derive-tag-scope", false);
    let provider = Provider::start(Vec::new());
    sandbox.configure(&config(&provider.url(), "").replace(
        "derive.in = [\"sessions\"]",
        "derive.in = [\"tag:a\"]\nderive.not_in = [\"tag:private\"]",
    ));
    let first = sandbox.session(FIRST);
    let second = sandbox.session(SECOND);
    declare_through_the_store(&sandbox, first, &["a"]);
    declare_through_the_store(&sandbox, second, &["a", "private"]);

    let all = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(all.code, 0, "{:?}", all.stderr);
    assert_eq!(
        all.stdout.lines().nth(1),
        Some("named: 2 entries by derive.in, 1 of them left out by derive.not_in")
    );
    assert_eq!(all.stdout.lines().nth(2), Some("due: 1 entries"));

    let run = sandbox.run(&[
        "derive",
        "--dry-run",
        &first.to_string(),
        &second.to_string(),
    ]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().nth(1),
        Some("named: 2 entries by derive.in, 1 of them left out by derive.not_in")
    );
    let entries: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("entry\t"))
        .collect();
    assert_eq!(
        entries,
        [
            format!("entry\t{first}\tdue\t-\t0"),
            format!("entry\t{second}\tleft out: derive.not_in\t-\t0"),
        ]
    );
}

#[test]
fn a_derive_in_value_that_is_no_tag_is_a_config_error_naming_the_key() {
    let sandbox = Sandbox::filled("derive-tag-bad", false);
    let provider = Provider::start(Vec::new());
    sandbox.configure(
        &config(&provider.url(), "")
            .replace("derive.in = [\"sessions\"]", "derive.in = [\"tag:Rust\"]"),
    );
    let text = fs::read_to_string(sandbox.dir.join("config/config.toml")).expect("config");
    let line = text
        .lines()
        .position(|line| line.starts_with("derive.in"))
        .expect("the key")
        + 1;

    let run = sandbox.run(&["derive", "--dry-run"]);

    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: {}/config/config.toml:{line}: derive.in must be a list of texts, each with a letter or a digit, and after tag: a tag's name, got \"tag:Rust\"",
            sandbox.dir.display()
        )]
    );
}

const TAGS_ON: &str = "derive.tags.enabled = true\nderive.tags.pick = [\"garden\", \"bilbo\"]\n";
const TAGS_LAST_LINE: &str =
    "</tags>\nWrite the tags for these notes now, one a line, at most 3, or (none).\n";

fn with_tags(url: &str, id: Id, extra: &str) -> String {
    only(id, url).replace(
        "derive.quiet = \"0s\"\n",
        &format!("derive.quiet = \"0s\"\n{extra}"),
    )
}

fn field(request: &Request, name: &str) -> Value {
    serde_json::from_str::<Value>(&body(request)).expect("a body")[name].clone()
}

fn asks_for_tags(request: &Request) -> bool {
    field(request, "system").as_str() == Some(bilbo_derive::tags::instruction::text())
}

fn user_text(request: &Request) -> String {
    field(request, "messages")[0]["content"]
        .as_str()
        .expect("a text")
        .to_owned()
}

fn standing(sandbox: &Sandbox, id: Id, name: &str) -> Option<bilbo_store::labels::Standing> {
    let store = sandbox.store();
    let meta = store.meta(id).expect("meta").expect("an entry");
    meta.labels.tags.standing(&Tag::new(name).expect("a tag"))
}

fn tagged_once(test: &str) -> (Sandbox, Provider, Id) {
    let sandbox = Sandbox::filled(test, false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer("garden"), answer("garden")]);
    sandbox.configure(&with_tags(&provider.url(), first, TAGS_ON));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 2);
    (sandbox, provider, first)
}

fn another_model(provider: &Provider, first: Id) -> String {
    with_tags(
        &provider.url(),
        first,
        &format!("{TAGS_ON}derive.tags.model = \"claude-test-2\"\n"),
    )
}

#[test]
fn a_run_writes_notes_then_tags_and_read_tag_and_recall_show_them() {
    let sandbox = Sandbox::filled("derive-tags-run", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer("garden")]);
    sandbox.configure(&with_tags(&provider.url(), first, TAGS_ON));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "entries 1, calls 2, notes 1, dropped 0, refused 0, failed 0"
    );
    assert_eq!(run.stderr, ["bilbo: tagged 1 entries".to_owned()]);
    let read = sandbox.run(&["read", &first.to_string(), "--budget", "256"]);
    assert!(
        read.stdout
            .lines()
            .any(|line| line == "label: tag:garden (model)"),
        "{}",
        read.stdout
    );
    let listed = sandbox.run(&["tag", "tag:garden"]);
    assert_eq!(listed.code, 0, "{:?}", listed.stderr);
    assert!(listed.stdout.contains("model"), "{}", listed.stdout);
    let found = sandbox.run(&["recall", "--in", "tag:garden", "garden", "fence"]);
    assert_eq!(found.code, 0, "{:?}", found.stderr);
    assert!(
        found.stdout.contains(&first.short().to_string()),
        "{}",
        found.stdout
    );
}

#[test]
fn the_tags_request_holds_the_notes_and_the_list_and_nothing_bilbo_adds_of_the_entry() {
    let sandbox = Sandbox::filled("derive-tags-request", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer("garden")]);
    sandbox.configure(&with_tags(&provider.url(), first, TAGS_ON));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let seen = requests(&provider);
    assert!(asks_for_tags(&seen[1]));
    let text = user_text(&seen[1]);
    assert!(text.starts_with("<notes>\n"), "{text}");
    assert!(text.ends_with(TAGS_LAST_LINE), "{text}");
    assert!(text.contains("bilbo\ngarden\n"), "{text}");
    for absent in [
        first.to_string(),
        format!("derive:{first}"),
        FIRST.to_owned(),
        "bilbo:".to_owned(),
    ] {
        assert!(!text.contains(&absent), "{absent}");
    }
}

#[test]
fn a_tag_removed_by_hand_is_not_declared_again() {
    let (sandbox, provider, first) = tagged_once("derive-tags-removed");
    let id = first.to_string();
    assert_eq!(sandbox.run(&["tag", &id, "--remove", "garden"]).code, 0);
    sandbox.configure(&another_model(&provider, first));
    let run = sandbox.run(&["derive", "--redo"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 3);
    assert!(asks_for_tags(&seen[2]));
    assert_eq!(
        standing(&sandbox, first, "garden"),
        Some(bilbo_store::labels::Standing::Removed)
    );
    let read = sandbox.run(&["read", &id, "--budget", "256"]);
    assert!(
        read.stdout
            .lines()
            .any(|line| line == "removed: tag:garden"),
        "{}",
        read.stdout
    );
    assert!(
        !read.stdout.contains("label: tag:garden"),
        "{}",
        read.stdout
    );
}

#[test]
fn a_persons_tag_is_left_by_redo() {
    let (sandbox, provider, first) = tagged_once("derive-tags-person");
    let id = first.to_string();
    assert_eq!(sandbox.run(&["tag", &id, "--add", "garden"]).code, 0);
    sandbox.configure(&another_model(&provider, first));
    let provider = Provider::start(vec![answer("(none)")]);
    sandbox.configure(&another_model(&provider, first));
    let run = sandbox.run(&["derive", "--redo"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 1);
    assert!(asks_for_tags(&seen[0]));
    assert_eq!(
        standing(&sandbox, first, "garden"),
        Some(bilbo_store::labels::Standing::Carried(
            bilbo_store::labels::By::Person
        ))
    );
    let read = sandbox.run(&["read", &id, "--budget", "256"]);
    assert!(
        read.stdout.lines().any(|line| line == "label: tag:garden"),
        "{}",
        read.stdout
    );
}

fn the_new_session(sandbox: &Sandbox) {
    sandbox.write(
        &format!("sessions/claude/p/{LONG}.jsonl"),
        &long_session(LONG, 2),
    );
    sandbox.seal();
}

fn plain_stdout(run: &Run) -> String {
    run.stdout
        .lines()
        .map(|line| {
            if line.starts_with("tokens in ") {
                line.split(", seconds").next().unwrap_or(line).to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn with_the_tags_off_the_requests_and_the_output_are_those_of_a_run_that_never_tagged() {
    let mut sent = Vec::new();
    for (name, tagged) in [("derive-tags-never", false), ("derive-tags-before", true)] {
        let sandbox = Sandbox::filled(name, false);
        let first = sandbox.session(FIRST);
        if tagged {
            let provider = Provider::start(vec![answer(NOTE_TEXT), answer("garden")]);
            sandbox.configure(&with_tags(&provider.url(), first, TAGS_ON));
            assert_eq!(sandbox.run(&["derive"]).code, 0);
        } else {
            let provider = Provider::start(vec![answer(NOTE_TEXT)]);
            sandbox.configure(&only(first, &provider.url()));
            assert_eq!(sandbox.run(&["derive"]).code, 0);
        }
        the_new_session(&sandbox);
        let provider =
            Provider::start(vec![answer("r1-2\nThe user asked about the garden fence.")]);
        let long = sandbox.session(LONG);
        sandbox.configure(&with_tags(
            &provider.url(),
            long,
            "derive.tags.pick = [\"garden\", \"bilbo\"]\n",
        ));
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        let seen = requests(&provider);
        assert_eq!(seen.len(), 1, "{name}");
        assert!(!asks_for_tags(&seen[0]), "{name}");
        sent.push((body(&seen[0]), plain_stdout(&run), run.stderr));
    }
    assert_eq!(sent[0], sent[1]);
}

#[test]
fn each_closed_gate_sends_nothing_with_the_tags_on() {
    let cases = [
        ("derive.enabled = false\n", "derive.enabled is false"),
        ("llm.provider = \"\"\n", "llm.provider is not set"),
        ("derive.in = []\n", "derive.in is empty"),
    ];
    for (gate, why) in cases {
        let sandbox = Sandbox::filled("derive-tags-gates", false);
        let first = sandbox.session(FIRST);
        let provider = Provider::start(vec![answer(NOTE_TEXT), answer("garden")]);
        sandbox.configure(&only(first, &provider.url()));
        assert_eq!(sandbox.run(&["derive"]).code, 0);
        assert_eq!(requests(&provider).len(), 1);
        let marker = sandbox.dir.join("key-ran");
        let key = script(&sandbox, &format!("touch {}\necho {KEY}", marker.display()));
        let mut text = with_tags(
            &provider.url(),
            first,
            &format!("{TAGS_ON}llm.anthropic.api.key_command = [\"/bin/sh\", \"{key}\"]\n"),
        );
        let leaf = gate.split(' ').next().expect("a key");
        text = text
            .lines()
            .filter(|line| !line.starts_with(leaf))
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
        text.push_str(gate);
        sandbox.configure(&text);
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{gate}: {:?}", run.stderr);
        assert_eq!(
            run.stderr,
            [format!("bilbo: derive is off: {why}")],
            "{gate}"
        );
        assert_eq!(requests(&provider).len(), 1, "{gate}");
        assert!(!marker.exists(), "{gate}");
        assert!(!sandbox.derive_dir().join("tagged").exists(), "{gate}");
    }
}

#[test]
fn a_tags_model_that_fails_its_prepare_exits_2_after_the_notes_are_kept_and_counted() {
    let sandbox = Sandbox::filled("derive-tags-prepare", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT)]);
    let extra = format!("{TAGS_ON}derive.tags.model = \"{KEY}\"\n");
    sandbox.configure(&with_tags(&provider.url(), first, &extra));
    let run = sandbox.run_with(&["derive", "--calls", "2"], &[("ANTHROPIC_API_KEY", KEY)]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "entries 1, calls 1, notes 1, dropped 0, refused 0, failed 0"
    );
    assert!(
        lines[1].starts_with("tokens in 100, tokens out 20, seconds "),
        "{}",
        lines[1]
    );
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some("bilbo: derive.tags.model holds the key")
    );
    assert!(!run.stdout.contains("PLUM") && !run.stderr.concat().contains("PLUM"));
    assert!(sandbox.notes_of(first).is_some());
}

#[test]
fn a_tags_model_that_fails_its_prepare_is_said_and_exits_2_though_the_closing_replay_fails() {
    let sandbox = Sandbox::filled("derive-tags-prepare-replay", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT)]);
    let extra = format!("{TAGS_ON}derive.tags.model = \"{KEY}\"\n");
    sandbox.configure(&with_tags(&provider.url(), first, &extra));
    let open_seg = stores::layout(&sandbox.vars()).state.join("open.seg");
    fs::set_permissions(&open_seg, fs::Permissions::from_mode(0o400)).expect("a mode");
    let run = sandbox.run(&["derive", "--calls", "2"]);
    fs::set_permissions(&open_seg, fs::Permissions::from_mode(0o600)).expect("a mode");
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "entries 1, calls 1, notes 1, dropped 0, refused 0, failed 0"
    );
    assert_eq!(lines.len(), 2, "{}", run.stdout);
    assert!(
        lines[1].starts_with("tokens in 100, tokens out 20, seconds "),
        "{}",
        lines[1]
    );
    let at = run
        .stderr
        .iter()
        .position(|line| line.starts_with("bilbo: cannot write ") && line.contains("open.seg"))
        .unwrap_or_else(|| panic!("{:?}", run.stderr));
    assert!(at + 1 < run.stderr.len(), "{:?}", run.stderr);
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some("bilbo: derive.tags.model holds the key")
    );
    assert!(
        spool_names(&sandbox)
            .iter()
            .any(|name| name.ends_with(".block"))
    );
}

fn walk_numbers(run: &Run) -> (String, String) {
    let at = |start: &str| {
        run.stdout
            .lines()
            .find(|line| line.starts_with(start))
            .expect("a line")
            .to_owned()
    };
    (at("this run:"), at("tags:"))
}

fn two_by_id(sandbox: &Sandbox) -> (Id, Id) {
    let ids = sandbox.sessions();
    (ids[ids.len() - 1], ids[ids.len() - 2])
}

fn in_both(url: &str, newer: Id, older: Id, extra: &str) -> String {
    only_ids(url, &[newer, older], extra)
}

#[test]
fn the_walk_passes_an_exhausted_asking_for_free_and_reaches_the_next_window() {
    let sandbox = Sandbox::filled("derive-walk-pass", false);
    let (a, b) = two_by_id(&sandbox);
    let tries = format!("{TAGS_ON}derive.tries = 1\n");
    let provider = Provider::start(vec![
        answer(NOTE_TEXT),
        refusal("policy"),
        answer(NOTE_TEXT),
    ]);
    sandbox.configure(&with_tags(&provider.url(), a, &tries));
    sandbox.run(&["derive"]);
    assert_eq!(requests(&provider).len(), 2);
    sandbox.configure(&in_both(&provider.url(), a, b, &tries));
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "1"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let (this_run, tags) = walk_numbers(&dry);
    assert!(
        this_run.starts_with("this run: 1 calls for 1 entries,"),
        "{this_run}"
    );
    assert!(tags.contains("; this run 0 calls,"), "{tags}");
    let run = sandbox.run(&["derive", "--calls", "1"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 3);
    assert!(!asks_for_tags(&seen[2]));
}

#[test]
fn the_dry_walk_counts_the_calls_the_run_then_makes() {
    let sandbox = Sandbox::filled("derive-walk-counts", false);
    let ids = sandbox.sessions();
    let (x, y, z) = (ids[ids.len() - 1], ids[ids.len() - 2], ids[ids.len() - 3]);
    let provider = Provider::start(vec![
        answer(NOTE_TEXT),
        answer(NOTE_TEXT),
        answer("garden"),
        answer("garden"),
        answer(NOTE_TEXT),
    ]);
    sandbox.configure(&only(y, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    assert_eq!(requests(&provider).len(), 1);
    let all = only_ids(&provider.url(), &[x, y, z], TAGS_ON);
    sandbox.configure(&all);
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "3"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let (this_run, tags) = walk_numbers(&dry);
    assert!(
        this_run.starts_with("this run: 1 calls for 1 entries,"),
        "{this_run}"
    );
    assert!(tags.contains("; this run 2 calls,"), "{tags}");
    let run = sandbox.run(&["derive", "--calls", "3"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 4);
    let kinds: Vec<bool> = seen[1..].iter().map(asks_for_tags).collect();
    assert_eq!(kinds, [false, true, true]);
}

#[test]
fn a_dry_run_adds_no_tags_call_for_an_entry_it_knows_has_no_note() {
    let sandbox = Sandbox::filled("derive-walk-nonote", false);
    let (a, b) = two_by_id(&sandbox);
    let tries = format!("{TAGS_ON}derive.tries = 1\n");
    let provider = Provider::start(vec![refusal("policy"), answer(NOTE_TEXT)]);
    sandbox.configure(&with_tags(&provider.url(), a, &tries));
    sandbox.run(&["derive"]);
    assert_eq!(requests(&provider).len(), 1);
    sandbox.configure(&in_both(&provider.url(), a, b, &tries));
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "1"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let (this_run, tags) = walk_numbers(&dry);
    assert!(
        this_run.starts_with("this run: 1 calls for 1 entries,"),
        "{this_run}"
    );
    assert!(tags.contains("; this run 0 calls,"), "{tags}");
    let run = sandbox.run(&["derive", "--calls", "1"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 2);
    assert!(!asks_for_tags(&seen[1]));
}

#[test]
fn a_name_added_to_the_list_asks_again_and_a_name_put_back_does_not() {
    let sandbox = Sandbox::filled("derive-tags-names", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer("a"), answer("b")]);
    let list = |names: &str| {
        with_tags(
            &provider.url(),
            first,
            &format!("derive.tags.enabled = true\nderive.tags.pick = [{names}]\n"),
        )
    };
    let mut counts = Vec::new();
    for names in ["\"a\"", "\"a\", \"b\"", "\"a\"", "\"a\", \"b\""] {
        sandbox.configure(&list(names));
        assert_eq!(sandbox.run(&["derive"]).code, 0);
        counts.push(requests(&provider).len());
    }
    assert_eq!(counts, [2, 3, 3, 3]);
    let seen = requests(&provider);
    assert!(asks_for_tags(&seen[1]) && asks_for_tags(&seen[2]));
    let (one, two) = (user_text(&seen[1]), user_text(&seen[2]));
    let notes = |text: &str| text.split("</notes>").next().unwrap_or("").to_owned();
    assert!(notes(&one).contains("The user asked about the garden fence."));
    assert_eq!(notes(&one), notes(&two));
}

#[test]
fn the_tags_file_lost_asks_once_more_and_takes_nothing_a_person_set() {
    let sandbox = Sandbox::filled("derive-tags-lost", false);
    let first = sandbox.session(FIRST);
    let id = first.to_string();
    let provider = Provider::start(vec![
        answer(NOTE_TEXT),
        answer("(none)"),
        answer("shed\ngarden"),
    ]);
    let extra = "derive.tags.enabled = true\nderive.tags.pick = [\"garden\", \"shed\", \"mine\"]\n";
    sandbox.configure(&with_tags(&provider.url(), first, extra));
    for (flag, name) in [("--add", "mine"), ("--add", "shed"), ("--remove", "shed")] {
        assert_eq!(sandbox.run(&["tag", &id, flag, name]).code, 0);
    }
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    assert_eq!(requests(&provider).len(), 2);
    assert!(sandbox.derive_dir().join("tagged").exists());
    fs::remove_file(sandbox.derive_dir().join("tagged")).expect("the file");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 3);
    assert_eq!(
        standing(&sandbox, first, "mine"),
        Some(Standing::Carried(By::Person))
    );
    assert_eq!(standing(&sandbox, first, "shed"), Some(Standing::Removed));
    assert_eq!(
        standing(&sandbox, first, "garden"),
        Some(Standing::Carried(By::Model))
    );
}

#[test]
fn a_tag_added_while_a_run_is_under_way_counts_from_the_next_run() {
    let _together = together();
    let sandbox = Sandbox::filled("derive-tags-underway", false);
    let mut pair = [
        (sandbox.session(FIRST), "carrot"),
        (sandbox.session(SECOND), "tomatoes"),
    ];
    pair.sort();
    let ((smaller, only_here), (larger, first_word)) = (pair[0], pair[1]);
    let (held, release) = Reply::held(&message(NOTE_TEXT));
    let provider = Provider::start(vec![held, answer(NOTE_TEXT)]);
    sandbox.configure(&only_ids(
        &provider.url(),
        &[larger, smaller],
        "derive.not_in = [\"tag:private\"]\n",
    ));
    let vars = sandbox.vars();
    let running = std::thread::spawn(move || {
        held_by_the_test();
        bilbo(vars, &["derive"])
    });
    waiting_for(&provider, 1);
    let added = sandbox.run(&["tag", &smaller.to_string(), "--add", "private"]);
    assert_eq!(added.code, 0, "{:?}", added.stderr);
    release.release();
    let run = running.join().expect("the run");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 2);
    assert!(body(&seen[0]).contains(first_word), "{}", body(&seen[0]));
    assert!(body(&seen[1]).contains(only_here), "{}", body(&seen[1]));
    let dry = sandbox.run(&["derive", "--dry-run", &smaller.to_string()]);
    assert!(
        dry.stdout
            .lines()
            .any(|line| line.starts_with(&format!("entry\t{smaller}\tleft out: derive.not_in"))),
        "{}",
        dry.stdout
    );
}

#[test]
fn an_entry_due_for_tags_alone_keeps_its_windows_charges() {
    let sandbox = Sandbox::filled("derive-tags-charges", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![refusal("policy"), answer(NOTE_TEXT), answer("garden")]);
    sandbox.configure(&only(first, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let copy = failed_file(&sandbox);
    assert!(!copy.is_empty());
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let path = sandbox.derive_dir().join("failed");
    fs::write(&path, &copy).expect("the copy back");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("a mode");
    sandbox.configure(&with_tags(&provider.url(), first, TAGS_ON));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 3);
    assert!(asks_for_tags(&requests(&provider)[2]));
    assert_eq!(failed_file(&sandbox), copy);
}

#[test]
fn a_tags_file_bilbo_may_not_open_fails_nothing_with_the_switch_off_or_the_list_empty() {
    for (extra, then) in [
        (
            "derive.tags.enabled = false\nderive.tags.pick = [\"garden\"]\n",
            true,
        ),
        ("derive.tags.enabled = true\nderive.tags.pick = []\n", true),
        (TAGS_ON, false),
    ] {
        let sandbox = Sandbox::filled("derive-tags-link", false);
        let first = sandbox.session(FIRST);
        let provider = Provider::start(vec![answer(NOTE_TEXT), answer("garden")]);
        sandbox.configure(&with_tags(&provider.url(), first, extra));
        fs::create_dir_all(sandbox.derive_dir()).expect("a folder");
        symlink(
            sandbox.dir.join("nowhere"),
            sandbox.derive_dir().join("tagged"),
        )
        .expect("a link");
        let run = sandbox.run(&["derive"]);
        if then {
            assert_eq!(run.code, 0, "{extra}: {:?}", run.stderr);
            assert!(sandbox.notes_of(first).is_some());
            assert!(
                run.stderr.iter().all(|line| !line.contains("tagged")),
                "{:?}",
                run.stderr
            );
        } else {
            assert_eq!(run.code, 1, "{:?}", run.stderr);
            assert!(
                run.stderr.iter().any(|line| line.contains("tagged")),
                "{:?}",
                run.stderr
            );
            assert!(requests(&provider).is_empty());
        }
    }
}

fn unbound_list(sandbox: &Sandbox, extra: &str) -> String {
    let mut items = with_instruction();
    items.insert(0, "--fallback-model");
    items.insert(1, "zz-value");
    tool_config(sandbox, &items).replace(
        "derive.quiet = \"0s\"\n",
        &format!("derive.quiet = \"0s\"\n{extra}"),
    )
}

const UNBOUND_MODEL: &str = "llm.anthropic.tool.arguments does not bind a call to its model: it needs \"--model\" followed by \"{model}\", and must not hold \"--fallback-model\"";

#[test]
fn a_tags_call_the_run_cannot_pay_is_not_refused_for_its_arguments() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-tags-unpaid", false);
    sandbox.configure(&unbound_list(
        &sandbox,
        "derive.tags.enabled = true\nderive.tags.pick = [\"garden\"]\n",
    ));
    let first = sandbox.session(FIRST).to_string();
    let run = sandbox.run(&["derive", "--calls", "1", &first]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stderr
            .iter()
            .all(|line| !line.contains("llm.anthropic.tool.arguments")),
        "{:?}",
        run.stderr
    );
    assert!(sandbox.notes_of(sandbox.session(FIRST)).is_some());
}

#[test]
fn a_list_that_does_not_bind_ends_the_run_at_the_first_tags_call_with_the_notes_kept() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-tags-unbound", false);
    sandbox.configure(&unbound_list(
        &sandbox,
        "derive.tags.enabled = true\nderive.tags.pick = [\"garden\"]\n",
    ));
    let first = sandbox.session(FIRST).to_string();
    let run = sandbox.run(&["derive", "--calls", "2", &first]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        run.stderr.last().map(String::as_str),
        Some(format!("bilbo: {UNBOUND_MODEL}").as_str())
    );
    assert!(sandbox.notes_of(sandbox.session(FIRST)).is_some());
    let off = Sandbox::filled("derive-tags-unbound-off", false);
    off.configure(&unbound_list(&off, ""));
    let first = off.session(FIRST).to_string();
    let run = off.run(&["derive", "--calls", "2", &first]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(off.notes_of(off.session(FIRST)).is_some());
}

#[test]
fn a_list_that_does_not_bind_asks_no_summary_and_writes_the_notes() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-summary-unbound", false);
    sandbox.configure(&unbound_list(&sandbox, SUMMARY_ON));
    let first = sandbox.session(FIRST).to_string();
    let second = sandbox.session(SECOND).to_string();
    let run = sandbox.run(&["derive", "--calls", "4", &first, &second]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let said: Vec<&String> = run
        .stderr
        .iter()
        .filter(|line| line.starts_with("bilbo: no summary is asked in this run: "))
        .collect();
    assert_eq!(said.len(), 1, "{:?}", run.stderr);
    assert!(said[0].contains(UNBOUND_MODEL), "{}", said[0]);
    assert_eq!(
        run.stderr
            .iter()
            .filter(|line| line.contains("llm.anthropic.tool.arguments"))
            .count(),
        1,
        "{:?}",
        run.stderr
    );
    let ran = fs::read_to_string(sandbox.dir.join("out/ran")).expect("the tool's marks");
    assert_eq!(ran.lines().count(), 2, "{ran}");
    assert!(sandbox.notes_of(sandbox.session(FIRST)).is_some());
    assert!(sandbox.notes_of(sandbox.session(SECOND)).is_some());
    assert!(spool_names(&sandbox).is_empty());
    assert!(!sandbox.derive_dir().join("summaries").exists());
}

#[test]
fn a_dry_run_with_a_list_that_does_not_bind_counts_no_summary_call() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-summary-unbound-dry", false);
    sandbox.configure(&unbound_list(&sandbox, SUMMARY_ON));
    let first = sandbox.session(FIRST).to_string();
    let second = sandbox.session(SECOND).to_string();
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "2", &first, &second]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    assert!(
        dry_lines(&dry, "this run:")[0].starts_with("this run: 2 calls for 2 entries, "),
        "{}",
        dry.stdout
    );
    let [line] = dry_lines(&dry, "summaries:")[..] else {
        panic!("one summaries line: {}", dry.stdout);
    };
    assert!(
        line.starts_with("summaries: 2 entries due; this run 0 calls, 0 tokens to send;"),
        "{line}"
    );
    let run = sandbox.run(&["derive", "--calls", "2", &first, &second]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let ran = fs::read_to_string(sandbox.dir.join("out/ran")).expect("the tool's marks");
    assert_eq!(ran.lines().count(), 2, "{ran}");
    assert!(sandbox.notes_of(sandbox.session(FIRST)).is_some());
    assert!(sandbox.notes_of(sandbox.session(SECOND)).is_some());
}

#[test]
fn a_tags_prepare_that_fails_after_passes_alone_says_the_passes_and_exits_2() {
    let sandbox = Sandbox::filled("derive-tags-passes", false);
    let (a, b) = two_by_id(&sandbox);
    let provider = Provider::start(vec![answer(NOTE_TEXT), refusal("policy")]);
    sandbox.configure(&only(b, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    sandbox.configure(&with_tags(&provider.url(), a, "derive.tries = 1\n"));
    sandbox.run(&["derive"]);
    assert_eq!(requests(&provider).len(), 2);
    let extra = format!("{TAGS_ON}derive.tries = 1\nderive.tags.model = \"{KEY}\"\n");
    sandbox.configure(&in_both(&provider.url(), a, b, &extra));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "entries 0, calls 0, notes 0, dropped 0, refused 0, failed 0"
    );
    assert_eq!(run.stderr.len(), 2, "{:?}", run.stderr);
    assert!(
        run.stderr[0].starts_with(&format!("bilbo: passed over {a} #r")),
        "{:?}",
        run.stderr
    );
    assert_eq!(run.stderr[1], "bilbo: derive.tags.model holds the key");
    assert_eq!(requests(&provider).len(), 2);
}

fn declared_by_a_model(sandbox: &Sandbox, id: Id, name: &str) {
    let picks = [Tag::new(name).expect("a tag")];
    let layout = stores::layout(&sandbox.vars());
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open the store");
    tag::declare(
        id,
        &Change::Model {
            picks: &picks,
            takes_back: false,
        },
        32,
        &mut store,
    )
    .expect("declare");
}

#[test]
fn a_dry_run_with_an_empty_list_still_gives_a_ref_its_tags_line() {
    let sandbox = Sandbox::filled("derive-tags-empty-ref", false);
    let first = sandbox.session(FIRST);
    declared_by_a_model(&sandbox, first, "garden");
    let provider = Provider::start(Vec::new());
    sandbox.configure(&with_tags(
        &provider.url(),
        first,
        "derive.tags.enabled = true\nderive.tags.pick = []\n",
    ));
    let run = sandbox.run(&["derive", "--dry-run", &first.to_string()]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert!(lines.contains(&"tags: none to pick from: derive.tags.pick is empty"));
    let entry = lines
        .iter()
        .position(|line| line.starts_with("entry\t"))
        .expect("an entry line");
    assert_eq!(
        lines[entry + 1],
        format!("tags\t{first}\tnot due: none to pick from\tgarden")
    );
    assert!(!sandbox.derive_dir().join("tagged").exists());
}

fn outcome_of(
    vars: HashMap<String, OsString>,
    args: &[&str],
    notice: Box<dyn FnMut(&bilbo_cli::console::message::Message)>,
) -> Result<bilbo_cli::console::outcome::Outcome, bilbo_cli::console::outcome::Failure> {
    let argv: Vec<OsString> = std::iter::once("bilbo")
        .chain(args.iter().copied())
        .map(OsString::from)
        .collect();
    let mut env = Env {
        var: Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::empty()),
        who: Who::Nobody,
        program: None,
        notice,
    };
    let settings = settings::load(
        &*env.var,
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .map_err(bilbo_cli::console::outcome::Failure::from);
    bilbo_cli::run_tuned(&bilbo_cli::bilbo(), &argv, &mut env, settings, &tune)
}

#[test]
fn a_replay_that_changes_no_tag_serves_a_refresh_that_found_derive_holding_the_store() {
    let _together = together();
    let sandbox = Sandbox::filled("derive-serves", false);
    let first = sandbox.session(FIRST);
    let (held, release) = Reply::held(&message("garden"));
    let provider = Provider::start(vec![answer(NOTE_TEXT), held]);
    sandbox.configure(&only(first, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    declare_through_the_store(&sandbox, first, &["garden"]);
    let followed = sandbox.dir.join("follow");
    fs::create_dir_all(&followed).expect("a folder");
    let file = followed.join("new.md");
    fs::write(&file, "# New\n\nA note kept by the pass derive serves.\n").expect("write");
    fs::File::options()
        .write(true)
        .open(&file)
        .expect("open")
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .expect("backdate");
    sandbox.configure(&format!(
        "{}\n[[follow]]\npath = {:?}\nkind = \"markdown\"\n",
        with_tags(&provider.url(), first, TAGS_ON),
        followed
    ));
    let layout = stores::layout(&sandbox.vars());
    let vars = sandbox.vars();
    let running = std::thread::spawn(move || {
        held_by_the_test();
        let bare = Rc::new(RefCell::new(None));
        let notice = {
            let (bare, vars) = (Rc::clone(&bare), vars.clone());
            Box::new(move |message: &bilbo_cli::console::message::Message| {
                let said = message.plain_lines().join("\n");
                if said.contains("torn frame") && bare.borrow().is_none() {
                    let vars = vars.clone();
                    let run =
                        bilbo_with(Rc::new(move |name| vars.get(name).cloned()), &["refresh"]);
                    *bare.borrow_mut() = Some((run.code, run.stdout, run.stderr));
                }
            })
        };
        let outcome = outcome_of(vars, &["derive"], notice).expect("the derive run");
        let said = outcome.stderr_lines(&bilbo_cli::console::view::View::PLAIN);
        let bare = bare.borrow().clone();
        (outcome.exit_code(), said, bare)
    });
    waiting_for(&provider, 2);
    let mut open_seg = fs::OpenOptions::new()
        .append(true)
        .open(layout.state.join("open.seg"))
        .expect("open.seg");
    io::Write::write_all(&mut open_seg, &[1, 0, 0]).expect("a torn tail");
    drop(open_seg);
    release.release();
    let (code, said, bare) = running.join().expect("the run");
    let (bare_code, bare_out, bare_err) = bare.expect("the bare run");
    assert_eq!(bare_code, 0, "{bare_err:?}");
    assert_eq!(bare_out, "");
    assert_eq!(
        bare_err,
        ["bilbo: another bilbo is writing the store; requested another pass over the follows"]
    );
    assert_eq!(code, 0, "{said:?}");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].starts_with("bilbo: ran the follows again for another bilbo: new 1, "),
        "{said:?}"
    );
    assert_eq!(
        fs::metadata(layout.state.join("store.again"))
            .expect("the flag")
            .len(),
        0
    );
    let store = Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("open");
    let label = CaptureLabel::file(&file.canonicalize().expect("canonical")).expect("a label");
    assert!(store.by_capture(&label).expect("a lookup").is_some());
    let asked = fs::read_to_string(sandbox.derive_dir().join("tagged")).expect("the file");
    assert!(
        asked.starts_with(&format!("answered\t{first}\t")),
        "{asked}"
    );
}

#[test]
fn a_replay_that_changes_nothing_does_no_index_work() {
    let sandbox = Sandbox::filled("derive-no-index-work", false);
    let first = sandbox.session(FIRST);
    let second = sandbox.session(SECOND);
    let provider = Provider::start(vec![answer(NOTE_TEXT)]);
    sandbox.configure(&only(first, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let (_, generation, records) = sandbox.notes_of(first).expect("notes");
    let block = bilbo_derive::spool::write(&bilbo_derive::spool::Block {
        parent: first,
        generation,
        generation_before: None,
        records_before: 0,
        opens: false,
        records: records
            .iter()
            .map(|record| record.trim_end_matches('\n').as_bytes().to_vec())
            .collect(),
    });
    let layout = stores::layout(&sandbox.vars());
    notes::noted(&layout, second, &[b"{\"text\":\"a note\"}\n".to_vec()]);
    spooled(&sandbox, &block);
    let behind = "bilbo: the index is behind the store, so labels may be missing; bilbo refresh brings it up to date";
    let run = sandbox.run(&["derive"]);
    assert_eq!((run.code, run.stderr), (1, vec![behind.to_owned()]));
    assert!(spool_names(&sandbox).is_empty());
    let again = sandbox.run(&["derive"]);
    assert_eq!((again.code, again.stderr), (1, vec![behind.to_owned()]));
    assert_eq!(requests(&provider).len(), 1);
}

const OWN_INSTRUCTION: &str = "Pick the tags for these notes, one a line, or (none).\n";

fn own_instruction(sandbox: &Sandbox) -> (PathBuf, String) {
    let file = sandbox.dir.join("tags-instruction.txt");
    fs::write(&file, OWN_INSTRUCTION).expect("an instruction");
    let line = format!(
        "derive.tags.instruction = {:?}\n",
        file.display().to_string()
    );
    (file, line)
}

#[test]
fn a_tags_instruction_of_the_settings_is_what_the_call_for_tags_sends() {
    let sandbox = Sandbox::filled("derive-tags-own-instruction", false);
    let first = sandbox.session(FIRST);
    let (_, line) = own_instruction(&sandbox);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer("garden")]);
    sandbox.configure(&with_tags(
        &provider.url(),
        first,
        &format!("{TAGS_ON}{line}"),
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 2);
    assert_eq!(
        field(&seen[0], "system").as_str(),
        Some(instruction::shipped()[0].1)
    );
    assert_eq!(field(&seen[1], "system").as_str(), Some(OWN_INSTRUCTION));
    assert!(!asks_for_tags(&seen[1]));
    assert_eq!(
        standing(&sandbox, first, "garden"),
        Some(Standing::Carried(By::Model))
    );
    let asked = fs::read_to_string(sandbox.derive_dir().join("tagged")).expect("the file");
    let id = instruction::id(OWN_INSTRUCTION);
    assert!(asked.contains(&format!("\t{id}\t")), "{asked}");
}

#[test]
fn a_bad_tags_instruction_file_is_a_config_error_only_while_the_tags_are_on() {
    let sandbox = Sandbox::filled("derive-tags-bad-instruction", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(NOTE_TEXT)]);
    let file = sandbox.dir.join("tags-instruction.txt");
    let name = file.display().to_string();
    let key = format!("derive.tags.instruction = {name:?}\n");
    sandbox.configure(&with_tags(
        &provider.url(),
        first,
        &format!("{TAGS_ON}{key}"),
    ));
    for (bytes, said) in [
        (
            None,
            format!(
                "bilbo: derive.tags.instruction: cannot read {name}: No such file or directory (os error 2)"
            ),
        ),
        (
            Some(b" \n".to_vec()),
            format!("bilbo: derive.tags.instruction: {name} is empty"),
        ),
        (
            Some(vec![0xff, 0xfe]),
            format!("bilbo: derive.tags.instruction: {name} is not UTF-8"),
        ),
    ] {
        if let Some(bytes) = bytes {
            fs::write(&file, bytes).expect("a file");
        }
        for args in [&["derive"][..], &["derive", "--dry-run"][..]] {
            let run = sandbox.run(args);
            assert_eq!((run.code, run.stderr), (2, vec![said.clone()]), "{args:?}");
        }
    }
    assert!(requests(&provider).is_empty());
    fs::remove_file(&file).expect("removed");
    for (extra, empty) in [
        (
            format!("derive.tags.enabled = false\nderive.tags.pick = [\"garden\"]\n{key}"),
            false,
        ),
        (
            format!("derive.tags.enabled = true\nderive.tags.pick = []\n{key}"),
            true,
        ),
    ] {
        let sandbox = Sandbox::filled("derive-tags-bad-instruction-off", false);
        let first = sandbox.session(FIRST);
        let provider = Provider::start(vec![answer(NOTE_TEXT)]);
        sandbox.configure(&with_tags(&provider.url(), first, &extra));
        let dry = sandbox.run(&["derive", "--dry-run"]);
        assert_eq!(dry.code, 0, "{:?}", dry.stderr);
        assert!(
            dry.stderr
                .iter()
                .all(|line| !line.contains("derive.tags.instruction"))
        );
        assert!(requests(&provider).is_empty());
        assert_eq!(
            dry.stdout
                .lines()
                .any(|line| line == "tags: none to pick from: derive.tags.pick is empty"),
            empty
        );
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        assert!(
            run.stderr
                .iter()
                .all(|line| !line.contains("derive.tags.instruction"))
        );
        assert!(sandbox.notes_of(first).is_some());
        assert_eq!(requests(&provider).len(), 1);
    }
    let sandbox = Sandbox::filled("derive-tags-bad-instruction-gate", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT)]);
    sandbox.configure(
        &with_tags(&provider.url(), first, &format!("{TAGS_ON}{key}"))
            .replace("derive.enabled = true", "derive.enabled = false"),
    );
    let run = sandbox.run(&["derive"]);
    assert_eq!(
        (run.code, run.stderr),
        (
            0,
            vec!["bilbo: derive is off: derive.enabled is false".to_owned()]
        )
    );
    let dry = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(
        (dry.code, dry.stderr),
        (
            2,
            vec![format!(
                "bilbo: derive.tags.instruction: cannot read {name}: No such file or directory (os error 2)"
            )]
        )
    );
    assert!(requests(&provider).is_empty());
}

#[test]
fn redo_asks_again_for_the_tags_another_instruction_gave_and_then_finds_nothing() {
    let (sandbox, provider, first) = tagged_once("derive-tags-redo-instruction");
    let (_, line) = own_instruction(&sandbox);
    sandbox.configure(&with_tags(
        &provider.url(),
        first,
        &format!("{TAGS_ON}{line}"),
    ));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    assert_eq!(requests(&provider).len(), 2);
    let redo = sandbox.run(&["derive", "--redo"]);
    assert_eq!(redo.code, 0, "{:?}", redo.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 3);
    assert_eq!(field(&seen[2], "system").as_str(), Some(OWN_INSTRUCTION));
    let asked = fs::read_to_string(sandbox.derive_dir().join("tagged")).expect("the file");
    assert!(
        asked.contains(&format!("\t{}\t", instruction::id(OWN_INSTRUCTION))),
        "{asked}"
    );
    assert_eq!(sandbox.run(&["derive", "--redo"]).code, 0);
    assert_eq!(requests(&provider).len(), 3);
}

#[test]
fn the_dry_run_names_the_tags_instruction_in_force() {
    let sandbox = Sandbox::filled("derive-tags-dry-instruction", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(Vec::new());
    sandbox.configure(&with_tags(&provider.url(), first, TAGS_ON));
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "1"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let (_, tags) = walk_numbers(&dry);
    assert!(
        tags.ends_with(&format!(
            "; instruction (built in, {})",
            bilbo_derive::tags::instruction::id()
        )),
        "{tags}"
    );
    let (file, line) = own_instruction(&sandbox);
    sandbox.configure(&with_tags(
        &provider.url(),
        first,
        &format!("{TAGS_ON}{line}"),
    ));
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "1"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let (_, tags) = walk_numbers(&dry);
    assert!(
        tags.ends_with(&format!(
            "; instruction ({}, {})",
            file.display(),
            instruction::id(OWN_INSTRUCTION)
        )),
        "{tags}"
    );
}

struct StateDir(PathBuf);

impl StateDir {
    fn new(test: &str) -> StateDir {
        let path = scratch::scratch_dir(test);
        fs::create_dir_all(&path).expect("a state folder");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("a mode");
        StateDir(path)
    }

    fn held(&self) -> bilbo_cli::derive::disk::Held {
        bilbo_cli::derive::disk::Folder::at(&self.0, fs::File::open(&self.0).expect("open"))
            .lock()
            .expect("a lock")
            .expect("held")
    }

    fn file(&self) -> PathBuf {
        self.0.join("derive/summaries")
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.0.join("derive"), fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_summaries_file_keeps_its_two_line_forms_and_reads_back() {
    use bilbo_derive::summary::asked::Asked;
    let state = StateDir::new("summaries_forms");
    let held = state.held();
    let (two, three) = (Id::from_bytes([2; 16]), Id::from_bytes([3; 16]));
    let mut summaries = held.summaries();
    summaries.pass(two, 6).expect("passed");
    summaries.charge(three, 7).expect("charged");
    assert_eq!(
        fs::read_to_string(state.file()).expect("the file"),
        format!("passed\t{two}\t0000000000000006\ncharged\t{three}\t0000000000000007\t1\n")
    );
    drop(summaries);
    drop(held);
    let held = state.held();
    let again = held.summaries();
    assert_eq!(again.passed(two).expect("read"), Some(6));
    assert_eq!(again.charges(three, 7).expect("read"), 1);
    assert_eq!(again.charges(three, 8).expect("read"), 0);
}

#[test]
fn a_charge_is_of_one_asking_and_a_pass_or_a_settle_takes_its_lines_out() {
    use bilbo_derive::summary::asked::Asked;
    let state = StateDir::new("summaries_askings");
    let held = state.held();
    let one = Id::from_bytes([1; 16]);
    let mut summaries = held.summaries();
    let no_temporary = || {
        assert!(!state.0.join("derive/summaries.tmp").exists());
    };
    summaries.charge(one, 7).expect("charged");
    no_temporary();
    summaries.charge(one, 7).expect("charged");
    no_temporary();
    assert_eq!(summaries.charges(one, 7).expect("read"), 2);
    summaries.charge(one, 8).expect("charged");
    no_temporary();
    assert_eq!(summaries.charges(one, 8).expect("read"), 1);
    assert_eq!(summaries.charges(one, 7).expect("read"), 0);
    summaries.pass(one, 8).expect("passed");
    no_temporary();
    assert_eq!(
        fs::read_to_string(state.file()).expect("the file"),
        format!("passed\t{one}\t0000000000000008\n")
    );
    summaries.settle(one).expect("settled");
    no_temporary();
    assert_eq!(fs::read_to_string(state.file()).expect("the file"), "");
    for _ in 0..10 {
        summaries.charge(one, 9).expect("charged");
    }
    assert_eq!(summaries.charges(one, 9).expect("read"), 9);
    assert_eq!(
        fs::read_to_string(state.file()).expect("the file"),
        format!("charged\t{one}\t0000000000000009\t9\n")
    );
}

#[test]
fn two_handles_of_the_summaries_file_under_one_lock_keep_each_others_lines() {
    use bilbo_derive::summary::asked::Asked;
    let state = StateDir::new("summaries_handles");
    let held = state.held();
    let (one, two) = (Id::from_bytes([1; 16]), Id::from_bytes([2; 16]));
    let mut first = held.summaries();
    let mut second = held.summaries();
    assert_eq!(first.charges(one, 7).expect("read"), 0);
    assert_eq!(second.charges(two, 8).expect("read"), 0);
    first.charge(one, 7).expect("charged");
    second.charge(two, 8).expect("charged");
    assert_eq!(
        fs::read_to_string(state.file()).expect("the file"),
        format!("charged\t{one}\t0000000000000007\t1\ncharged\t{two}\t0000000000000008\t1\n")
    );
    assert_eq!(first.charges(two, 8).expect("read"), 1);
    assert_eq!(second.charges(one, 7).expect("read"), 1);
}

#[test]
fn a_write_of_the_summaries_file_that_failed_leaves_nothing_of_its_change_behind() {
    use bilbo_derive::summary::asked::Asked;
    let state = StateDir::new("summaries_failed");
    let held = state.held();
    let one = Id::from_bytes([1; 16]);
    let mut summaries = held.summaries();
    summaries.charge(one, 7).expect("charged");
    let derive = state.0.join("derive");
    fs::set_permissions(&derive, fs::Permissions::from_mode(0o500)).expect("a mode");
    summaries.charge(one, 7).expect_err("a write that fails");
    fs::set_permissions(&derive, fs::Permissions::from_mode(0o700)).expect("a mode");
    assert_eq!(summaries.charges(one, 7).expect("read"), 1);
    summaries.charge(one, 7).expect("charged");
    assert_eq!(
        fs::read_to_string(state.file()).expect("the file"),
        format!("charged\t{one}\t0000000000000007\t2\n")
    );
}

#[test]
fn a_summaries_file_that_is_no_private_file_is_refused_by_both_readers_and_no_file_is_the_empty_one()
 {
    use bilbo_derive::summary::asked::Asked;
    let state = StateDir::new("summaries_private");
    drop(state.held());
    let handle = fs::File::open(&state.0).expect("a folder");
    let path = state.file();
    let sentence = format!(
        "{} is not a private file of this user in a folder of this user",
        path.display()
    );
    let entry = Id::from_bytes([1; 16]);
    let mine = state.0.join("derive/mine");
    fs::write(&mine, "x").expect("a file");
    fs::set_permissions(&mine, fs::Permissions::from_mode(0o600)).expect("a mode");
    for case in [
        "no file",
        "a link",
        "a second name",
        "a file open to others",
    ] {
        match case {
            "a link" => symlink(&mine, &path).expect("a link"),
            "a second name" => fs::hard_link(&mine, &path).expect("a name"),
            "a file open to others" => {
                fs::write(&path, "x").expect("a file");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("a mode");
            }
            _ => {}
        }
        let held = state.held();
        let readers = [
            held.summaries(),
            bilbo_cli::derive::disk::summaries_reader(&handle, &state.0),
        ];
        for reader in readers {
            let found = reader.passed(entry);
            if case == "no file" {
                assert_eq!(found.expect("no error"), None, "{case}");
            } else {
                assert_eq!(found.expect_err(case).to_string(), sentence, "{case}");
            }
        }
        if case != "no file" {
            fs::remove_file(&path).expect("removed");
        }
    }
}

#[test]
fn a_replay_that_kept_a_summary_alone_brings_the_index_up_to_the_store() {
    let sandbox = Sandbox::filled("derive-summarised", false);
    let provider = Provider::start(Vec::new());
    let store = sandbox.store();
    let (changes, _) = store.changes(&Cursor::start()).expect("changes");
    let file = changes
        .iter()
        .map(bilbo_store::entries::Change::id)
        .find(|id| {
            store
                .meta(*id)
                .expect("meta")
                .is_some_and(|meta| meta.kind == "markdown")
        })
        .expect("a file");
    let label = store
        .meta(file)
        .expect("meta")
        .and_then(|meta| meta.labels.capture)
        .expect("a label");
    let head = store.by_capture(&label).expect("a lookup").expect("a head");
    drop(store);
    assert_eq!(sandbox.run(&["reindex"]).code, 0);
    sandbox.configure(&only(file, &provider.url()));
    let block = bilbo_derive::summary::block::write(&bilbo_derive::summary::block::Block {
        entry: file,
        about: bilbo_derive::summary::asked::About {
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: None,
        },
        asking: 5,
        form: bilbo_derive::summary::block::Form::Answer,
        instruction: "i".to_owned(),
        provider: "anthropic".to_owned(),
        model: "claude-test-1".to_owned(),
        answered: None,
        made: 1_700_000_100,
        sent: 10,
        tokens_in: None,
        tokens_out: None,
        text: "The garden notes.".to_owned(),
    });
    let folder = sandbox.derive_dir().join("spool");
    fs::create_dir_all(&folder).expect("a folder");
    let kept = folder.join("00000000000000000001.summary");
    fs::write(&kept, block).expect("a block");
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o600)).expect("a mode");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stderr
            .iter()
            .any(|line| line == "bilbo: summarised 1 entries"),
        "{:?}",
        run.stderr
    );
    assert!(spool_names(&sandbox).is_empty());
    let dry = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!((dry.code, dry.stderr), (0, Vec::<String>::new()));
}

#[test]
fn a_summarys_block_waits_in_the_spool_under_a_name_of_its_own() {
    use bilbo_derive::spool::Spool;
    let state = StateDir::new("summaries_spool");
    let held = state.held();
    let mut summary = held.summary_blocks().expect("a spool");
    summary.keep(b"one").expect("kept");
    summary.keep(b"two").expect("kept");
    let mut plain = held.blocks().expect("a spool");
    plain.keep(b"three").expect("kept");
    let names = |extension: &str| -> Vec<String> {
        let mut found: Vec<String> = fs::read_dir(state.0.join("derive/spool"))
            .expect("a folder")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(extension))
            .collect();
        found.sort();
        found
    };
    let mut every = names("");
    every.sort();
    assert_eq!(
        every,
        [
            "00000000000000000001.block",
            "00000000000000000001.summary",
            "00000000000000000002.summary"
        ]
    );
    let mut older = held.blocks().expect("a spool");
    let listed = older.list().expect("a list");
    assert_eq!(
        listed
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["00000000000000000001.block"]
    );
    older.remove(&listed[0].0).expect("removed");
    assert!(older.is_empty().expect("empty"));
    assert_eq!(names(".summary").len(), 2);
    let mut again = held.summary_blocks().expect("a spool");
    let kept = again.list().expect("a list");
    assert_eq!(
        kept.iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "00000000000000000001.summary",
            "00000000000000000002.summary"
        ]
    );
    again.keep(b"three").expect("kept");
    assert_eq!(
        names(".summary").last().map(String::as_str),
        Some("00000000000000000003.summary")
    );
}

#[test]
fn a_replay_that_finds_its_summary_already_kept_brings_the_index_up_to_the_store() {
    let sandbox = Sandbox::filled("derive-found", false);
    let provider = Provider::start(Vec::new());
    let store = sandbox.store();
    let (changes, _) = store.changes(&Cursor::start()).expect("changes");
    let file = changes
        .iter()
        .map(bilbo_store::entries::Change::id)
        .find(|id| {
            store
                .meta(*id)
                .expect("meta")
                .is_some_and(|meta| meta.kind == "markdown")
        })
        .expect("a file");
    let label = store
        .meta(file)
        .expect("meta")
        .and_then(|meta| meta.labels.capture)
        .expect("a label");
    let head = store.by_capture(&label).expect("a lookup").expect("a head");
    drop(store);
    assert_eq!(sandbox.run(&["reindex"]).code, 0);
    sandbox.configure(&only(file, &provider.url()));
    let block = bilbo_derive::summary::block::write(&bilbo_derive::summary::block::Block {
        entry: file,
        about: bilbo_derive::summary::asked::About {
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: None,
        },
        asking: 5,
        form: bilbo_derive::summary::block::Form::Answer,
        instruction: "i".to_owned(),
        provider: "anthropic".to_owned(),
        model: "claude-test-1".to_owned(),
        answered: None,
        made: 1_700_000_100,
        sent: 10,
        tokens_in: None,
        tokens_out: None,
        text: "The garden notes.".to_owned(),
    });
    let mut record =
        bilbo_derive::summary::record::write(&bilbo_derive::summary::record::Summary {
            text: "The garden notes.".to_owned(),
            instruction: "i".to_owned(),
            provider: "anthropic".to_owned(),
            model: "claude-test-1".to_owned(),
            answered: None,
            made: 1_700_000_100,
            entry: file,
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: None,
            sent: 10,
            tokens_in: None,
            tokens_out: None,
        })
        .into_bytes();
    record.push(b'\n');
    let summary_label =
        CaptureLabel::new(bilbo_derive::summary::label(file).to_string()).expect("a label");
    let layout = stores::layout(&sandbox.vars());
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open the store");
    store
        .capture(&Capture {
            label: &summary_label,
            kind: bilbo_derive::summary::KIND,
            id: None,
            id_label: None,
            stamp: None,
            bytes: &record,
            ends: &[record.len()],
            tags: &Tags::default(),
        })
        .expect("capture");
    store.close().expect("close");
    let folder = sandbox.derive_dir().join("spool");
    fs::create_dir_all(&folder).expect("a folder");
    let kept = folder.join("00000000000000000001.summary");
    fs::write(&kept, block).expect("a block");
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o600)).expect("a mode");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stderr.iter().all(|line| !line.contains("summarised")),
        "{:?}",
        run.stderr
    );
    assert!(spool_names(&sandbox).is_empty());
    let dry = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!((dry.code, dry.stderr), (0, Vec::<String>::new()));
}

#[test]
fn two_handles_of_the_summarys_spool_under_one_lock_number_their_blocks_apart() {
    use bilbo_derive::spool::Spool;
    let state = StateDir::new("summaries_numbers");
    let held = state.held();
    let mut first = held.summary_blocks().expect("a spool");
    let mut second = held.summary_blocks().expect("a spool");
    first.keep(b"one").expect("kept");
    second.keep(b"two").expect("kept");
    let folder = state.0.join("derive/spool");
    let mut names: Vec<String> = fs::read_dir(&folder)
        .expect("a folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "00000000000000000001.summary",
            "00000000000000000002.summary"
        ]
    );
    assert_eq!(
        fs::read(folder.join("00000000000000000001.summary")).expect("read"),
        b"one"
    );
    assert_eq!(
        fs::read(folder.join("00000000000000000002.summary")).expect("read"),
        b"two"
    );
}

const SUMMARY_ON: &str = "derive.summary.enabled = true\nderive.summary.least = 0\n";
const SUMMARY_TEXT: &str = "The garden fence was asked about.";

fn summarising(url: &str, id: Id, extra: &str) -> String {
    with_tags(url, id, &format!("{SUMMARY_ON}{extra}"))
}

fn asks_for_summary(request: &Request) -> bool {
    field(request, "system").as_str() == Some(bilbo_derive::summary::instruction::text())
}

fn summary_records(sandbox: &Sandbox, id: Id) -> Vec<String> {
    let store = sandbox.store();
    let Some(head) = store
        .by_capture(&bilbo_derive::summary::label(id))
        .expect("a lookup")
    else {
        return Vec::new();
    };
    store
        .read(
            head.id,
            head.generation,
            RecordRange {
                first: 1,
                count: head.records,
            },
        )
        .expect("records")
        .iter()
        .map(|record| String::from_utf8_lossy(record).into_owned())
        .collect()
}

fn head_of(sandbox: &Sandbox, id: Id) -> bilbo_store::entries::Head {
    let store = sandbox.store();
    let label = store
        .meta(id)
        .expect("meta")
        .and_then(|meta| meta.labels.capture)
        .expect("a label");
    store.by_capture(&label).expect("a lookup").expect("a head")
}

#[test]
fn a_run_writes_notes_then_a_summary_and_keeps_it_beside_the_entry() {
    let sandbox = Sandbox::filled("derive-summary-run", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&summarising(&provider.url(), first, ""));
    let run = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 2, notes 1, dropped 0, refused 0, failed 0\n"),
        "{}",
        run.stdout
    );
    assert_eq!(run.stderr, ["bilbo: summarised 1 entries".to_owned()]);
    let records = summary_records(&sandbox, first);
    assert_eq!(records.len(), 1);
    let held = bilbo_derive::summary::record::read(records[0].as_bytes()).expect("a summary");
    let head = head_of(&sandbox, first);
    assert_eq!(held.text, SUMMARY_TEXT);
    assert_eq!(held.instruction, bilbo_derive::summary::instruction::id());
    assert_eq!(
        (held.provider.as_str(), held.model.as_str()),
        ("anthropic", "claude-test-1")
    );
    assert_eq!(
        (held.generation, held.records, held.content),
        (head.generation, head.records, head.mark.hash)
    );
    let notes = held.notes.expect("the notes it was written from");
    assert_eq!((notes.generation, notes.records), (Generation(1), 2));
    let store = sandbox.store();
    let entry = store
        .by_capture(&bilbo_derive::summary::label(first))
        .expect("a lookup")
        .expect("a head");
    let meta = store.meta(entry.id).expect("meta").expect("an entry");
    assert_eq!(meta.kind, bilbo_derive::summary::KIND);
    assert_eq!(meta.labels.id, None);
    drop(store);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 2);
    assert!(asks_for_summary(&seen[1]) && !asks_for_summary(&seen[0]));
    assert!(spool_names(&sandbox).is_empty());
}

#[test]
fn a_markdown_file_derive_in_names_gets_a_summary_of_its_text() {
    let sandbox = Sandbox::filled("derive-summary-file", false);
    let file = sandbox.of_kind("markdown", false)[0];
    let provider = Provider::start(vec![answer(SUMMARY_TEXT)]);
    sandbox.configure(&summarising(&provider.url(), file, "").replace(
        &format!("derive.in = [\"{file}\"]"),
        "derive.in = [\"notes\"]",
    ));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stdout.starts_with("entries 1, calls 1, notes 0,"),
        "{}",
        run.stdout
    );
    let seen = requests(&provider);
    assert_eq!(seen.len(), 1);
    let text = user_text(&seen[0]);
    assert!(text.starts_with("<entry>\n"), "{text}");
    assert!(
        text.ends_with("</entry>\nWrite the summary now, in at most 200 characters.\n"),
        "{text}"
    );
    let first_line = String::from_utf8_lossy(NOTES[0].1)
        .lines()
        .next()
        .expect("a line")
        .to_owned();
    assert!(text.contains(&first_line), "{text}");
    let records = summary_records(&sandbox, file);
    assert_eq!(records.len(), 1);
    let held = bilbo_derive::summary::record::read(records[0].as_bytes()).expect("a summary");
    assert!(held.notes.is_none());
}

#[test]
fn the_summarys_request_holds_the_notes_and_nothing_bilbo_adds_of_the_entry() {
    let sandbox = Sandbox::filled("derive-summary-request", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&summarising(&provider.url(), first, ""));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let seen = requests(&provider);
    assert!(asks_for_summary(&seen[1]));
    let text = user_text(&seen[1]);
    assert_eq!(
        text,
        "<notes>\nr1-7 2026-11-07\nThe user asked about the garden fence.\n\n</notes>\nWrite the summary now, in at most 200 characters.\n"
    );
    assert!(text.starts_with("<notes>\n"), "{text}");
    assert!(
        text.ends_with("</notes>\nWrite the summary now, in at most 200 characters.\n"),
        "{text}"
    );
    assert!(
        text.contains("The user asked about the garden fence."),
        "{text}"
    );
    for absent in [
        first.to_string(),
        format!("derive:{first}"),
        FIRST.to_owned(),
        "file:".to_owned(),
        sandbox.dir.display().to_string(),
    ] {
        assert!(!text.contains(&absent), "{absent}");
    }
}

const GARDEN_ONLY: &str = "derive.tags.enabled = true\nderive.tags.pick = [\"garden\"]\n";

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("a folder");
    for entry in fs::read_dir(from).expect("a folder").flatten() {
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let meta = fs::metadata(&source).expect("metadata");
        if meta.is_dir() {
            copy_tree(&source, &target);
        } else {
            fs::copy(&source, &target).expect("a copy");
            fs::File::options()
                .write(true)
                .open(&target)
                .expect("open")
                .set_modified(meta.modified().expect("a time"))
                .expect("a time");
        }
        fs::set_permissions(&target, meta.permissions()).expect("a mode");
    }
}

#[test]
fn with_the_summary_off_the_requests_and_the_output_are_those_of_a_run_that_never_summarised() {
    let sandbox = Sandbox::filled("derive-summary-off", false);
    let ids = sandbox.sessions();
    let kept = sandbox.dir.join("pristine");
    for folder in ["store", "state", "cache"] {
        copy_tree(&sandbox.dir.join(folder), &kept.join(folder));
    }
    let mut sent = Vec::new();
    for on in [false, true] {
        if on {
            for folder in ["store", "state", "cache"] {
                fs::remove_dir_all(sandbox.dir.join(folder)).expect("removed");
                copy_tree(&kept.join(folder), &sandbox.dir.join(folder));
            }
        }
        let name = if on { "on" } else { "off" };
        let mut replies = Vec::new();
        for _ in &ids {
            replies.extend([answer(NOTE_TEXT), answer("garden")]);
            if on {
                replies.push(answer(SUMMARY_TEXT));
            }
        }
        let provider = Provider::start(replies);
        let extra = if on {
            format!("{GARDEN_ONLY}{SUMMARY_ON}")
        } else {
            GARDEN_ONLY.to_owned()
        };
        sandbox.configure(&only_ids(&provider.url(), &ids, &extra));
        assert_eq!(sandbox.sessions(), ids, "{name}");
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{name}: {:?}", run.stderr);
        let seen = requests(&provider);
        if on {
            assert_eq!(seen.len(), 9, "{name}");
        } else {
            assert_eq!(seen.len(), 6, "{name}");
            assert!(
                run.stdout
                    .starts_with("entries 3, calls 6, notes 3, dropped 0, refused 0, failed 0\n"),
                "{}",
                run.stdout
            );
            assert!(seen.iter().all(|request| !asks_for_summary(request)));
            let store = sandbox.store();
            for id in &ids {
                assert!(
                    store
                        .by_capture(&bilbo_derive::summary::label(*id))
                        .expect("a lookup")
                        .is_none(),
                    "{id}"
                );
            }
            drop(store);
            assert!(sandbox.of_kind("summary", false).is_empty());
            assert!(!sandbox.derive_dir().join("summaries").exists());
            assert!(
                spool_names(&sandbox)
                    .iter()
                    .all(|name| !name.ends_with(".summary"))
            );
            let log = fs::read_to_string(sandbox.derive_dir().join("log")).expect("a log");
            assert!(log.lines().all(|line| !line.starts_with("u1\t")));
        }
        let windows: Vec<String> = seen
            .iter()
            .filter(|request| !asks_for_tags(request) && !asks_for_summary(request))
            .map(body)
            .collect();
        let tags: Vec<String> = seen
            .iter()
            .filter(|request| asks_for_tags(request))
            .map(body)
            .collect();
        sent.push((windows, tags));
    }
    assert_eq!(sent[0], sent[1]);
}

#[test]
fn each_closed_gate_sends_nothing_with_the_summary_on() {
    let cases = [
        ("derive.enabled = false\n", "derive.enabled is false"),
        ("llm.provider = \"\"\n", "llm.provider is not set"),
        ("derive.in = []\n", "derive.in is empty"),
    ];
    for (gate, why) in cases {
        let sandbox = Sandbox::filled("derive-summary-gates", false);
        let first = sandbox.session(FIRST);
        let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
        let mut text = summarising(&provider.url(), first, "");
        let leaf = gate.split(' ').next().expect("a key");
        text = text
            .lines()
            .filter(|line| !line.starts_with(leaf))
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
        text.push_str(gate);
        sandbox.configure(&text);
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{gate}: {:?}", run.stderr);
        assert_eq!(
            run.stderr,
            [format!("bilbo: derive is off: {why}")],
            "{gate}"
        );
        assert!(requests(&provider).is_empty(), "{gate}");
    }
}

fn dry_lines<'a>(run: &'a Run, start: &str) -> Vec<&'a str> {
    run.stdout
        .lines()
        .filter(|line| line.starts_with(start))
        .collect()
}

#[test]
fn the_dry_walk_counts_the_summaries_the_run_then_makes() {
    let sandbox = Sandbox::filled("derive-summary-walk", false);
    let ids = sandbox.sessions();
    let provider = Provider::start(vec![
        answer(NOTE_TEXT),
        answer(SUMMARY_TEXT),
        answer(NOTE_TEXT),
        answer(SUMMARY_TEXT),
    ]);
    sandbox.configure(&only_ids(&provider.url(), &ids, SUMMARY_ON));
    let dry = sandbox.run(&["derive", "--dry-run", "--calls", "4"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    assert_eq!(dry_lines(&dry, "due:"), ["due: 3 entries"]);
    assert!(
        dry_lines(&dry, "this run:")[0].starts_with("this run: 2 calls for 2 entries, "),
        "{}",
        dry.stdout
    );
    assert_eq!(
        dry_lines(&dry, "summaries:"),
        [format!(
            "summaries: 3 entries due; this run 2 calls, 0 tokens to send; instruction (built in, {})",
            bilbo_derive::summary::instruction::id()
        )]
    );
    let run = sandbox.run(&["derive", "--calls", "4"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    let kinds: Vec<bool> = seen.iter().map(asks_for_summary).collect();
    assert_eq!(kinds, [false, true, false, true]);
    assert!(
        run.stdout.starts_with("entries 2, calls 4, notes 2,"),
        "{}",
        run.stdout
    );
    assert!(
        run.stderr
            .contains(&"bilbo: summarised 2 entries".to_owned()),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_dry_run_counts_the_calls_and_the_tokens_of_summaries_with_no_window() {
    let sandbox = Sandbox::filled("derive-summary-no-window", false);
    let first = sandbox.session(FIRST);
    let file = sandbox.of_kind("markdown", false)[0];
    let provider = Provider::start(vec![
        answer(NOTE_TEXT),
        answer(SUMMARY_TEXT),
        answer(SUMMARY_TEXT),
    ]);
    sandbox.configure(&only(first, &provider.url()));
    let notes = sandbox.run(&["derive"]);
    assert_eq!(notes.code, 0, "{:?}", notes.stderr);
    sandbox.configure(&only_ids(&provider.url(), &[first, file], SUMMARY_ON));
    let dry = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let [line] = dry_lines(&dry, "summaries:")[..] else {
        panic!("one summaries line: {}", dry.stdout);
    };
    let head = "summaries: 2 entries due; this run 2 calls, ";
    let tail = format!(
        " tokens to send; instruction (built in, {})",
        bilbo_derive::summary::instruction::id()
    );
    let counted = line
        .strip_prefix(head)
        .and_then(|rest| rest.strip_suffix(tail.as_str()))
        .unwrap_or_else(|| panic!("{line}"));
    let tokens: u64 = counted.parse().expect("a count");
    assert!(tokens > 0, "{line}");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let log = fs::read_to_string(sandbox.derive_dir().join("log")).expect("a log");
    let sent: Vec<u64> = log
        .lines()
        .filter(|line| line.starts_with("u1\t"))
        .filter(|line| line.split('\t').nth(2) == Some("written"))
        .map(|line| {
            line.split('\t')
                .nth(9)
                .expect("sent")
                .parse()
                .expect("a count")
        })
        .collect();
    assert_eq!(sent.len(), 2, "{log}");
    assert_eq!(sent.iter().sum::<u64>(), tokens, "{log}");
}

#[test]
fn with_the_key_false_a_dry_run_prints_no_line_of_the_summary() {
    let sandbox = Sandbox::filled("derive-summary-dry", false);
    let ids = sandbox.sessions();
    let provider = Provider::start(Vec::new());
    let url = provider.url();
    sandbox.configure(&only_ids(&url, &ids, ""));
    let off = sandbox.run(&["derive", "--dry-run"]);
    let off_four = sandbox.run(&["derive", "--dry-run", "--calls", "4"]);
    for line in off.stdout.lines() {
        assert!(!line.starts_with("summaries:") && !line.starts_with("summary\t"));
    }
    sandbox.configure(&only_ids(&url, &ids, SUMMARY_ON));
    let on = sandbox.run(&["derive", "--dry-run"]);
    let kept: Vec<&str> = on
        .stdout
        .lines()
        .filter(|line| !line.starts_with("summaries:") && !line.starts_with("summary\t"))
        .collect();
    assert_eq!(kept.join("\n") + "\n", off.stdout);
    assert!(on.stdout.lines().any(|line| line.starts_with("summaries:")));
    let on_four = sandbox.run(&["derive", "--dry-run", "--calls", "4"]);
    let apart = |run: &Run| -> Vec<String> {
        run.stdout
            .lines()
            .filter(|line| {
                !line.starts_with("this run:")
                    && !line.starts_with("summaries:")
                    && !line.starts_with("summary\t")
            })
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(apart(&on_four), apart(&off_four));
    assert!(
        on_four
            .stdout
            .lines()
            .any(|line| line.starts_with("provider:")),
        "{}",
        on_four.stdout
    );
    assert!(
        dry_lines(&off_four, "this run:")[0].starts_with("this run: 3 calls for 3 entries,"),
        "{}",
        off_four.stdout
    );
    assert!(
        dry_lines(&on_four, "this run:")[0].starts_with("this run: 2 calls for 2 entries,"),
        "{}",
        on_four.stdout
    );
}

#[test]
fn a_dry_run_gives_each_ref_its_summary_line() {
    let sandbox = Sandbox::filled("derive-summary-refs", true);
    let first = sandbox.session(FIRST);
    let markdown = sandbox.of_kind("markdown", false)[0];
    let subagent = sandbox.of_kind("session", true)[0];
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&summarising(&provider.url(), first, ""));
    let run = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let dry = sandbox.run(&[
        "derive",
        "--dry-run",
        &first.to_string(),
        &markdown.to_string(),
        &subagent.to_string(),
    ]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    let refs: Vec<&str> = dry
        .stdout
        .lines()
        .filter(|line| line.starts_with("entry\t") || line.starts_with("summary\t"))
        .collect();
    assert_eq!(refs.len(), 6, "{}", dry.stdout);
    assert!(refs[..3].iter().all(|line| line.starts_with("entry\t")));
    assert!(refs[3..].iter().all(|line| line.starts_with("summary\t")));
    let ids: Vec<String> = [first, markdown, subagent]
        .iter()
        .map(ToString::to_string)
        .collect();
    for (at, id) in ids.iter().enumerate() {
        assert!(refs[at].starts_with(&format!("entry\t{id}\t")), "{refs:?}");
    }
    assert_eq!(
        &refs[3..],
        [
            format!("summary\t{first}\twritten"),
            format!("summary\t{markdown}\tnot due: left out: not named by derive.in"),
            format!("summary\t{subagent}\tnot due: left out: belongs to another entry"),
        ]
    );
}

#[test]
fn a_file_named_on_the_command_line_writes_its_summary_and_warns_of_nothing() {
    let sandbox = Sandbox::filled("derive-summary-named", false);
    let file = sandbox.of_kind("markdown", false)[0];
    let provider = Provider::start(vec![answer(SUMMARY_TEXT), answer(SUMMARY_TEXT)]);
    let both = |extra: &str, named: &str| {
        config(&provider.url(), extra).replace(
            "derive.in = [\"sessions\"]",
            &format!("derive.in = [{named}]"),
        )
    };
    sandbox.configure(&both(SUMMARY_ON, "\"sessions\", \"notes\""));
    let run = sandbox.run(&["derive", &file.to_string()]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, ["bilbo: summarised 1 entries".to_owned()]);
    assert_eq!(requests(&provider).len(), 1);
    let again = sandbox.run(&["derive", &file.to_string()]);
    assert_eq!((again.code, again.stderr), (0, Vec::<String>::new()));
    assert_eq!(requests(&provider).len(), 1);

    let sandbox = Sandbox::filled("derive-summary-named-off", false);
    let file = sandbox.of_kind("markdown", false)[0];
    sandbox.configure(&both("", "\"sessions\", \"notes\""));
    let off = sandbox.run(&["derive", &file.to_string()]);
    assert_eq!(off.code, 1);
    assert_eq!(requests(&provider).len(), 1);
    assert_eq!(
        off.stderr,
        [format!(
            "bilbo: {file} is of the kind markdown, which has no instruction"
        )]
    );
    sandbox.configure(&both(SUMMARY_ON, "\"sessions\""));
    let unnamed = sandbox.run(&["derive", &file.to_string()]);
    assert_eq!(unnamed.code, 1);
    assert_eq!(requests(&provider).len(), 1);
    assert_eq!(
        unnamed.stderr,
        [format!("bilbo: {file} is not named by derive.in")]
    );
}

fn summary_config_of_file(url: &str, file: Id, extra: &str) -> String {
    summarising(url, file, extra).replace(
        &format!("derive.in = [\"{file}\"]"),
        "derive.in = [\"notes\"]",
    )
}

#[test]
fn an_edited_file_is_summarised_again_and_an_untouched_one_is_not() {
    let sandbox = Sandbox::filled("derive-summary-edit", false);
    let file = sandbox.of_kind("markdown", false)[0];
    let provider = Provider::start(vec![
        answer(SUMMARY_TEXT),
        answer("The garden log changed."),
    ]);
    sandbox.configure(&summary_config_of_file(&provider.url(), file, ""));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    assert_eq!(requests(&provider).len(), 1);
    let again = sandbox.run(&["derive"]);
    assert_eq!(again.code, 0, "{:?}", again.stderr);
    assert!(
        again.stdout.starts_with("entries 0, calls 0,"),
        "{}",
        again.stdout
    );
    assert_eq!(requests(&provider).len(), 1);
    let mut edited = NOTES[0].1.to_vec();
    edited.extend_from_slice(b"\n## Fences\n\nThe fence was mended in spring.\n");
    sandbox.write("notes/sections.md", &edited);
    sandbox.seal();
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 2);
    assert_eq!(summary_records(&sandbox, file).len(), 2);
    let dry = sandbox.run(&["derive", "--dry-run", &file.to_string()]);
    assert_eq!(
        dry_lines(&dry, "summary\t"),
        [format!("summary\t{file}\twritten")]
    );
}

#[test]
fn a_summary_refused_in_three_runs_is_passed_over_and_redo_asks_again() {
    let sandbox = Sandbox::filled("derive-summary-passed", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT)]);
    sandbox.configure(&only(first, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let provider = Provider::start(vec![
        refusal("cyber"),
        refusal("cyber"),
        refusal("cyber"),
        answer(SUMMARY_TEXT),
    ]);
    sandbox.configure(&summarising(&provider.url(), first, ""));
    for _ in 0..3 {
        let run = sandbox.run(&["derive"]);
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        assert_eq!(
            run.stderr,
            [format!("bilbo: refused the summary of {first}: cyber")]
        );
    }
    assert_eq!(requests(&provider).len(), 3);
    let fourth = sandbox.run(&["derive"]);
    assert_eq!(fourth.code, 0, "{:?}", fourth.stderr);
    assert_eq!(requests(&provider).len(), 3);
    assert_eq!(
        fourth.stderr,
        [format!(
            "bilbo: passed over the summary of {first}: refused or failed in 3 runs"
        )]
    );
    let dry = sandbox.run(&["derive", "--dry-run", &first.to_string()]);
    assert_eq!(
        dry_lines(&dry, "summary\t"),
        [format!("summary\t{first}\tpassed over")]
    );
    let redo = sandbox.run(&["derive", "--redo", &first.to_string()]);
    assert_eq!(redo.code, 0, "{:?}", redo.stderr);
    assert_eq!(requests(&provider).len(), 4);
}

#[test]
fn an_answer_with_no_summary_is_said_in_bilbos_words_and_exits_1_with_the_notes_kept() {
    let sandbox = Sandbox::filled("derive-summary-empty", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer("")]);
    sandbox.configure(&summarising(&provider.url(), first, ""));
    let run = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert!(
        run.stdout
            .starts_with("entries 1, calls 2, notes 1, dropped 0, refused 0, failed 1\n"),
        "{}",
        run.stdout
    );
    assert!(
        run.stderr.contains(&format!(
            "bilbo: call failed for the summary of {first}: the answer holds no summary bilbo can keep"
        )),
        "{:?}",
        run.stderr
    );
    let (_, _, records) = sandbox.notes_of(first).expect("the notes");
    assert!(
        records[0].contains("The user asked about the garden fence."),
        "{records:?}"
    );
    assert!(summary_records(&sandbox, first).is_empty());
    let charged = fs::read_to_string(sandbox.derive_dir().join("summaries")).expect("the file");
    assert_eq!(charged.lines().count(), 1, "{charged}");
    let fields: Vec<&str> = charged.trim_end_matches('\n').split('\t').collect();
    assert_eq!(fields.len(), 4, "{charged}");
    assert_eq!(fields[0], "charged");
    assert_eq!(fields[1], first.to_string());
    let head = head_of(&sandbox, first);
    let (note, generation, _) = sandbox.notes_of(first).expect("the notes");
    let paragraph = bilbo_cli::notes().paragraph;
    let store = sandbox.store();
    let stored = bilbo_derive::plan::held(&store, first)
        .expect("a lookup")
        .expect("notes");
    let mut digest = None;
    for record in store
        .read(
            note,
            generation,
            RecordRange {
                first: 1,
                count: stored.records,
            },
        )
        .expect("records")
    {
        let bare = bilbo_derive::spool::bare(&record);
        if paragraph(bare).is_some() {
            digest = Some(bilbo_derive::tags::asked::digest(digest, bare));
        }
    }
    drop(store);
    let sent = user_text(&requests(&provider)[1]);
    let asking = bilbo_derive::summary::asked::asking(
        first,
        &bilbo_derive::summary::asked::About {
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: digest,
        },
        "anthropic",
        "claude-test-1",
        &bilbo_derive::summary::instruction::id(),
        &sent,
    );
    assert_eq!(fields[2], format!("{asking:016x}"), "{charged}");
    assert_eq!(fields[3], "1");
}

#[test]
fn a_run_that_died_after_the_call_keeps_its_summary_at_the_next_run() {
    let sandbox = Sandbox::filled("derive-summary-died", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT)]);
    sandbox.configure(&only(first, &provider.url()));
    assert_eq!(sandbox.run(&["derive"]).code, 0);
    let head = head_of(&sandbox, first);
    let (note, generation, _) = sandbox.notes_of(first).expect("notes");
    let paragraph = bilbo_cli::notes().paragraph;
    let store = sandbox.store();
    let stored = bilbo_derive::plan::held(&store, first)
        .expect("a lookup")
        .expect("notes");
    let mut digest = None;
    for record in store
        .read(
            note,
            generation,
            RecordRange {
                first: 1,
                count: stored.records,
            },
        )
        .expect("records")
    {
        let bare = bilbo_derive::spool::bare(&record);
        if paragraph(bare).is_some() {
            digest = Some(bilbo_derive::tags::asked::digest(digest, bare));
        }
    }
    drop(store);
    let block = bilbo_derive::summary::block::write(&bilbo_derive::summary::block::Block {
        entry: first,
        about: bilbo_derive::summary::asked::About {
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: digest,
        },
        asking: 5,
        form: bilbo_derive::summary::block::Form::Answer,
        instruction: bilbo_derive::summary::instruction::id(),
        provider: "anthropic".to_owned(),
        model: "claude-test-1".to_owned(),
        answered: None,
        made: 1_700_000_100,
        sent: 10,
        tokens_in: None,
        tokens_out: None,
        text: SUMMARY_TEXT.to_owned(),
    });
    let folder = sandbox.derive_dir().join("spool");
    fs::create_dir_all(&folder).expect("a folder");
    let kept = folder.join("00000000000000000001.summary");
    fs::write(&kept, block).expect("a block");
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o600)).expect("a mode");
    assert_eq!(spool_names(&sandbox), ["00000000000000000001.summary"]);
    sandbox.configure(&summarising(&provider.url(), first, ""));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 1);
    assert_eq!(run.stderr, ["bilbo: summarised 1 entries".to_owned()]);
    let records = summary_records(&sandbox, first);
    assert_eq!(records.len(), 1);
    let held = bilbo_derive::summary::record::read(records[0].as_bytes()).expect("a summary");
    assert_eq!(held.text, SUMMARY_TEXT);
    assert_eq!(held.instruction, bilbo_derive::summary::instruction::id());
    assert_eq!(
        (held.provider.as_str(), held.model.as_str(), held.made),
        ("anthropic", "claude-test-1", 1_700_000_100)
    );
    assert_eq!(
        (held.generation, held.records, held.content),
        (head.generation, head.records, head.mark.hash)
    );
    assert_eq!(
        held.notes,
        Some(bilbo_derive::summary::record::Notes {
            generation,
            records: stored.records,
            digest: digest.expect("a paragraph"),
        })
    );
    assert!(spool_names(&sandbox).is_empty());
    let again = sandbox.run(&["derive"]);
    assert_eq!((again.code, again.stderr), (0, Vec::<String>::new()));
    assert_eq!(requests(&provider).len(), 1);
}

#[test]
fn a_summary_instruction_of_the_settings_is_what_the_call_sends_and_the_dry_run_names() {
    let sandbox = Sandbox::filled("derive-summary-own-instruction", false);
    let first = sandbox.session(FIRST);
    let file = sandbox.dir.join("summary-instruction.txt");
    fs::write(&file, "Sum it up.\n").expect("an instruction");
    let line = format!(
        "derive.summary.instruction = {:?}\n",
        file.display().to_string()
    );
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&summarising(&provider.url(), first, &line));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let seen = requests(&provider);
    assert_eq!(seen.len(), 2);
    assert_eq!(field(&seen[1], "system").as_str(), Some("Sum it up.\n"));
    let records = summary_records(&sandbox, first);
    let held = bilbo_derive::summary::record::read(records[0].as_bytes()).expect("a summary");
    let id = instruction::id("Sum it up.\n");
    assert_eq!(held.instruction, id);
    let dry = sandbox.run(&["derive", "--dry-run"]);
    let summaries = dry_lines(&dry, "summaries:");
    assert!(
        summaries[0].ends_with(&format!("instruction ({}, {id})", file.display())),
        "{summaries:?}"
    );
}

#[test]
fn a_bad_summary_instruction_file_is_a_config_error_only_while_the_summary_is_on() {
    let sandbox = Sandbox::filled("derive-summary-bad-instruction", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT)]);
    let file = sandbox.dir.join("missing-summary.txt");
    let name = file.display().to_string();
    let key = format!("derive.summary.instruction = {name:?}\n");
    sandbox.configure(&summarising(&provider.url(), first, &key));
    let said = format!(
        "bilbo: derive.summary.instruction: cannot read {name}: No such file or directory (os error 2)"
    );
    for args in [&["derive"][..], &["derive", "--dry-run"][..]] {
        let run = sandbox.run(args);
        assert_eq!((run.code, run.stderr), (2, vec![said.clone()]), "{args:?}");
    }
    assert!(requests(&provider).is_empty());
    sandbox.configure(
        &summarising(&provider.url(), first, &key)
            .replace("derive.enabled = true", "derive.enabled = false"),
    );
    let run = sandbox.run(&["derive"]);
    assert_eq!(
        (run.code, run.stderr),
        (
            0,
            vec!["bilbo: derive is off: derive.enabled is false".to_owned()]
        )
    );
    sandbox.configure(&with_tags(
        &provider.url(),
        first,
        &format!("derive.summary.enabled = false\n{key}"),
    ));
    for args in [&["derive"][..], &["derive", "--dry-run"][..]] {
        let run = sandbox.run(args);
        assert_eq!(run.code, 0, "{args:?} {:?}", run.stderr);
        assert!(run.stderr.iter().all(|line| !line.contains("summary")));
    }
}

#[test]
fn the_log_gets_a_u1_line_and_no_text_of_the_summary() {
    let sandbox = Sandbox::filled("derive-summary-log", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&summarising(&provider.url(), first, ""));
    assert_eq!(sandbox.run(&["derive", &first.to_string()]).code, 0);
    let log = fs::read_to_string(sandbox.derive_dir().join("log")).expect("a log");
    let d1: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("d1\t"))
        .collect();
    let u1: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("u1\t"))
        .collect();
    assert_eq!((d1.len(), u1.len()), (1, 1), "{log}");
    let fields: Vec<&str> = u1[0].split('\t').collect();
    assert_eq!(fields.len(), 16, "{}", u1[0]);
    assert_eq!(fields[2], "written");
    assert_eq!(fields[4], first.to_string());
    assert!(!log.contains(SUMMARY_TEXT) && !log.contains("garden fence"));
}

#[test]
fn a_summaries_file_bilbo_may_not_open_fails_nothing_with_the_switch_off() {
    for on in [false, true] {
        let sandbox = Sandbox::filled("derive-summary-link", false);
        let first = sandbox.session(FIRST);
        let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
        if on {
            sandbox.configure(&summarising(&provider.url(), first, ""));
        } else {
            sandbox.configure(&only(first, &provider.url()));
        }
        fs::create_dir_all(sandbox.derive_dir()).expect("a folder");
        symlink(
            sandbox.dir.join("nowhere"),
            sandbox.derive_dir().join("summaries"),
        )
        .expect("a link");
        let run = sandbox.run(&["derive"]);
        let dry = sandbox.run(&["derive", "--dry-run"]);
        if on {
            assert_eq!(run.code, 1, "{:?}", run.stderr);
            assert!(
                run.stderr.iter().any(|line| line.contains("summaries")),
                "{:?}",
                run.stderr
            );
        } else {
            assert_eq!(run.code, 0, "{:?}", run.stderr);
            assert_eq!(dry.code, 0, "{:?}", dry.stderr);
            assert!(sandbox.notes_of(first).is_some());
            for said in run.stderr.iter().chain(dry.stderr.iter()) {
                assert!(!said.contains("summaries"), "{said}");
            }
        }
    }
}

#[test]
fn a_config_that_does_not_name_the_key_summarises() {
    let sandbox = Sandbox::filled("derive-summary-default", false);
    let first = sandbox.session(FIRST);
    let provider = Provider::start(Vec::new());
    sandbox.as_given(&only(first, &provider.url()));
    let file = fs::read_to_string(sandbox.dir.join("config/config.toml")).expect("a config");
    assert!(!file.contains("derive.summary"), "{file}");
    let config = sandbox.run(&["config"]);
    assert!(
        config
            .stdout
            .lines()
            .any(|line| line == "derive.summary.enabled = true (default)"),
        "{}",
        config.stdout
    );
    let dry = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    assert!(!dry_lines(&dry, "summaries: ").is_empty(), "{}", dry.stdout);
}

fn four_queries(sandbox: &Sandbox, file: Id) -> Vec<(u8, String, Vec<String>, String)> {
    let log = sandbox.dir.join("state/bilbo/queries.log");
    let id = file.to_string();
    let mut found = Vec::new();
    for args in [
        vec!["recall", "garden"],
        vec!["recall", "--in", id.as_str(), "garden"],
        vec!["recall", "--in", "notes", "garden"],
        vec!["recall", "--in", "summary", "garden"],
    ] {
        let before = fs::read_to_string(&log).map_or(0, |held| held.len());
        let run = sandbox.run(&args);
        let written = fs::read_to_string(&log).expect("the query log");
        let line = written[before..].to_owned();
        assert_eq!(line.matches('\n').count(), 1, "{args:?}: {line:?}");
        assert!(line.ends_with('\n'), "{args:?}: {line:?}");
        let mut fields: Vec<String> = line[..line.len() - 1]
            .split('\t')
            .map(str::to_owned)
            .collect();
        assert_eq!(fields[0], "q4", "{args:?}");
        fields[1] = "<time>".to_owned();
        found.push((run.code, run.stdout, run.stderr, fields.join("\t") + "\n"));
    }
    found
}

const DIGEST_KEYS: [(&str, &str); 12] = [
    ("BILBO_DIGEST_MARKDOWN_MOST", "6"),
    ("BILBO_DIGEST_SESSION_MOST", "6"),
    ("BILBO_DIGEST_NOTE_MOST", "6"),
    ("BILBO_DIGEST_MARKDOWN_FLOOR", "0"),
    ("BILBO_DIGEST_SESSION_FLOOR", "0"),
    ("BILBO_DIGEST_NOTE_FLOOR", "0"),
    ("BILBO_DIGEST_SESSION_OLD_AFTER", "36500d"),
    ("BILBO_DIGEST_SESSION_SUBAGENTS", "false"),
    ("BILBO_DIGEST_SESSION_PARTS", "false"),
    ("BILBO_DIGEST_NOTE_WIDTH", "300"),
    ("BILBO_DIGEST_SUMMARY_ENABLED", "true"),
    ("BILBO_SEARCH_WORKERS", "2"),
];

fn digest_prompt(sandbox: &Sandbox, session: &str, prompt: &str) -> Run {
    let mut vars = sandbox.vars();
    for (name, value) in DIGEST_KEYS {
        vars.insert(name.to_owned(), OsString::from(value));
    }
    let hook = format!("{{\"session_id\":\"{session}\",\"prompt\":\"{prompt}\"}}").into_bytes();
    let var: Var = Rc::new(move |name| vars.get(name).cloned());
    let said = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&said);
    let mut env = Env {
        var,
        stdin: Box::new(io::Cursor::new(hook)),
        who: Who::Nobody,
        program: None,
        notice: Box::new(move |message| sink.borrow_mut().extend(message.plain_lines())),
    };
    let settings = settings::load(
        &*env.var,
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
        &bilbo_cli::provider_keys(),
    )
    .map_err(bilbo_cli::console::outcome::Failure::from);
    let argv: Vec<OsString> = ["bilbo", "digest"].iter().map(OsString::from).collect();
    let (code, stdout, lines) =
        match bilbo_cli::run_tuned(&bilbo_cli::bilbo(), &argv, &mut env, settings, &tune) {
            Ok(outcome) => (
                outcome.exit_code(),
                outcome.stdout(&bilbo_cli::console::view::View::PLAIN),
                outcome.stderr_lines(&bilbo_cli::console::view::View::PLAIN),
            ),
            Err(failure) => (
                failure.exit_code(),
                String::new(),
                failure.stderr_lines(&bilbo_cli::console::view::View::PLAIN),
            ),
        };
    let mut stderr = said.borrow().clone();
    stderr.extend(lines);
    Run {
        code,
        stdout,
        stderr,
    }
}

#[test]
fn recall_prints_the_same_bytes_once_a_summary_is_kept_and_no_index_holds_it() {
    let sandbox = Sandbox::filled("derive-summary-recall", false);
    let file = sandbox.of_kind("markdown", false)[0];
    assert_eq!(sandbox.run(&["reindex"]).code, 0);
    let provider = Provider::start(vec![answer("Zucchini trellis.")]);
    sandbox.configure(&summary_config_of_file(&provider.url(), file, ""));
    let before = four_queries(&sandbox, file);
    let reindexed = sandbox.run(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{:?}", reindexed.stderr);
    assert!(reindexed.stderr.is_empty(), "{:?}", reindexed.stderr);
    let counts = reindexed.stdout.lines().next().expect("a line").to_owned();
    assert!(
        counts.contains("unread 0") && counts.contains("failed 0"),
        "{counts}"
    );
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 1);
    assert_eq!(run.stderr, ["bilbo: summarised 1 entries".to_owned()]);
    let after = four_queries(&sandbox, file);
    assert_eq!(before, after);
    let none = sandbox.run(&["recall", "zucchini", "trellis"]);
    assert_eq!(
        (none.code, none.stderr.clone()),
        (
            1,
            vec![
                "bilbo: searched 4 entries".to_owned(),
                "bilbo: nothing matched".to_owned()
            ]
        )
    );
    assert!(
        none.stdout.lines().all(|line| !line.starts_with("=== ")),
        "{}",
        none.stdout
    );
    let again = sandbox.run(&["reindex"]);
    assert_eq!(again.code, 0, "{:?}", again.stderr);
    assert_eq!(
        again.stdout.lines().count(),
        reindexed.stdout.lines().count()
    );
    assert_eq!(again.stdout.lines().next().expect("a line"), counts);
    assert!(again.stderr.is_empty(), "{:?}", again.stderr);
    let session = "00000000-0000-4000-8000-000000000077";
    let digest = digest_prompt(&sandbox, session, "zucchini trellis");
    assert_eq!((digest.code, digest.stderr), (0, Vec::<String>::new()));
    assert_eq!(digest.stdout, "");
}

fn seal_follow(sandbox: &Sandbox) {
    stores::backdate(&stores::layout(&sandbox.vars()).state, 120);
    let run = sandbox.run(&["refresh"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
}

#[test]
fn a_file_that_is_gone_has_its_summary_retired_at_the_next_run() {
    let sandbox = Sandbox::new("derive-summary-gone");
    sandbox.write(
        "notes/first.md",
        b"# The first\n\nThe carrots were thinned in the north bed.\n",
    );
    sandbox.write(
        "notes/second.md",
        b"# The second\n\nThe tomatoes were staked along the south wall.\n",
    );
    let provider = Provider::start(vec![answer(SUMMARY_TEXT), answer(SUMMARY_TEXT)]);
    let folder = sandbox.path("notes");
    let text = format!(
        "{}\n[[follow]]\npath = \"{folder}\"\nkind = \"markdown\"\n",
        config(&provider.url(), SUMMARY_ON)
            .replace("derive.in = [\"sessions\"]", "derive.in = [\"notes\"]")
    );
    sandbox.configure(&text);
    let refreshed = sandbox.run(&["refresh"]);
    assert_eq!(refreshed.code, 0, "{:?}", refreshed.stderr);
    seal_follow(&sandbox);
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(requests(&provider).len(), 2);
    let files = sandbox.of_kind("markdown", false);
    assert_eq!(files.len(), 2);
    let gone_id = files
        .iter()
        .copied()
        .find(|id| {
            sandbox
                .store()
                .meta(*id)
                .expect("meta")
                .and_then(|meta| meta.labels.capture)
                .is_some_and(|label| label.as_str().ends_with("first.md"))
        })
        .expect("the first file");
    let kept_id = files
        .iter()
        .copied()
        .find(|id| *id != gone_id)
        .expect("the other");
    let kept_records = summary_records(&sandbox, kept_id);
    assert_eq!(kept_records.len(), 1);
    fs::remove_file(sandbox.dir.join("notes/first.md")).expect("removed");
    let withdrawn = sandbox.run(&["refresh"]);
    assert_eq!(withdrawn.code, 0, "{:?}", withdrawn.stderr);
    seal_follow(&sandbox);
    let next = sandbox.run(&["derive"]);
    assert_eq!(next.code, 0, "{:?}", next.stderr);
    assert!(next.stderr.is_empty(), "{:?}", next.stderr);
    assert_eq!(requests(&provider).len(), 2);
    let records = summary_records(&sandbox, gone_id);
    assert_eq!(records.len(), 2, "{records:?}");
    let first = bilbo_derive::summary::record::read(records[0].as_bytes()).expect("a summary");
    assert_eq!(first.text, SUMMARY_TEXT);
    assert!(bilbo_derive::summary::record::read(records[1].as_bytes()).is_none());
    let withdrawn = head_of(&sandbox, gone_id);
    assert_eq!(withdrawn.records, 0);
    let retired: Value = serde_json::from_str(&records[1]).expect("a JSON record");
    let summary = &retired["summary"];
    assert!(summary["retired"].as_u64().is_some(), "{retired}");
    assert_eq!(
        summary.as_object().expect("an object").len(),
        2,
        "{retired}"
    );
    assert_eq!(
        summary["of"],
        json!({
            "entry": gone_id.to_string(),
            "generation": withdrawn.generation.0,
            "records": 0,
            "content": format!("{:032x}", withdrawn.mark.hash),
        })
    );
    assert_eq!(summary_records(&sandbox, kept_id), kept_records);
    let dry = sandbox.run(&["derive", "--dry-run", &gone_id.to_string()]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    assert!(dry.stderr.is_empty(), "{:?}", dry.stderr);
    assert_eq!(
        dry_lines(&dry, "summary\t"),
        [format!("summary\t{gone_id}\tnot due: short")]
    );
    let after = sandbox.run(&["derive"]);
    assert_eq!(after.code, 0, "{:?}", after.stderr);
    assert_eq!(summary_records(&sandbox, gone_id), records);
    assert_eq!(summary_records(&sandbox, kept_id), kept_records);
    let surviving = summary_records(&sandbox, kept_id);
    assert!(
        bilbo_derive::summary::record::read(surviving.last().expect("a record").as_bytes())
            .is_some()
    );
}

#[test]
fn a_replay_that_fails_after_a_capture_still_brings_the_index_up_to_the_store() {
    let sandbox = Sandbox::filled("derive-replay-fails-after", false);
    let first = sandbox.session(FIRST);
    let file = sandbox.of_kind("markdown", false)[0];
    let provider = Provider::start(Vec::new());
    assert_eq!(sandbox.run(&["reindex"]).code, 0);
    sandbox.configure(&only(first, &provider.url()));
    spooled(&sandbox, &kept_block(&sandbox, first));
    let head = head_of(&sandbox, file);
    let pass = bilbo_derive::summary::block::write(&bilbo_derive::summary::block::Block {
        entry: file,
        about: bilbo_derive::summary::asked::About {
            generation: head.generation,
            records: head.records,
            content: head.mark.hash,
            notes: None,
        },
        asking: 5,
        form: bilbo_derive::summary::block::Form::Pass,
        instruction: bilbo_derive::summary::instruction::id(),
        provider: "anthropic".to_owned(),
        model: String::new(),
        answered: None,
        made: 1_700_000_100,
        sent: 0,
        tokens_in: None,
        tokens_out: None,
        text: String::new(),
    });
    let kept = sandbox
        .derive_dir()
        .join("spool/00000000000000000001.summary");
    fs::write(&kept, pass).expect("a block");
    fs::set_permissions(&kept, fs::Permissions::from_mode(0o600)).expect("a mode");
    let temporary = sandbox.derive_dir().join("summaries.tmp");
    fs::create_dir(&temporary).expect("a folder");
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    let last = run.stderr.last().expect("a line");
    assert!(
        last.starts_with("bilbo: cannot remove ") && last.contains("summaries.tmp"),
        "{last}"
    );
    assert_eq!(spool_names(&sandbox), ["00000000000000000001.summary"]);
    assert!(sandbox.notes_of(first).is_some());
    fs::remove_dir(&temporary).expect("removed");
    let dry = sandbox.run(&["derive", "--dry-run"]);
    assert_eq!(dry.code, 0, "{:?}", dry.stderr);
    assert!(dry.stderr.iter().all(|line| !line.contains("behind")));
    let again = sandbox.run(&["derive"]);
    assert_eq!(again.code, 0, "{:?}", again.stderr);
    assert!(spool_names(&sandbox).is_empty());
    let held = fs::read_to_string(sandbox.derive_dir().join("summaries")).expect("the file");
    assert!(held.starts_with("passed\t"), "{held}");
}

fn forget_entry(
    sandbox: &Sandbox,
    id: Id,
    hook: Option<bilbo_store::segments::Hook>,
) -> Result<bilbo_entries::forget::Output, bilbo_entries::forget::Error> {
    let layout = stores::layout(&sandbox.vars());
    let options = Options {
        hook,
        ..Options::default()
    };
    let mut store = Store::open(&layout, options, &mut |_| {}).expect("open the store");
    let request = bilbo_entries::forget::Request {
        targets: vec![bilbo_entries::read::refs::Target::Entry(id)],
        listed: 5,
    };
    bilbo_entries::forget::forget(&request, &bilbo_cli::labelled, &mut store)
}

fn refreshed(sandbox: &Sandbox) {
    let run = sandbox.run(&["refresh"]);
    assert!(run.code <= 1, "{:?}", run.stderr);
}

const FIRST_TEXT: &str = "purslane";

fn mentions_the_first(request: &Request) -> bool {
    body(request).contains(FIRST_TEXT)
}

#[test]
fn derive_sends_nothing_for_what_was_forgotten() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-forgotten", false);
    let first = sandbox.session(FIRST);
    let second = sandbox.session(SECOND);
    let both = [first, second];
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&only_ids(&provider.url(), &both, SUMMARY_ON));
    let made = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(made.code, 0, "{:?}", made.stderr);
    assert!(sandbox.notes_of(first).is_some());
    assert_eq!(summary_records(&sandbox, first).len(), 1);
    assert!(requests(&provider).iter().any(mentions_the_first));

    forget_entry(&sandbox, first, None).expect("forgotten");
    refreshed(&sandbox);
    let control = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&only_ids(&control.url(), &both, SUMMARY_ON));
    let run = sandbox.run(&["derive"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let sent = requests(&control);
    assert!(!sent.is_empty());
    assert!(!sent.iter().any(mentions_the_first));
    assert!(sandbox.notes_of(second).is_some());
    assert!(sandbox.notes_of(first).is_none());

    let other = Sandbox::filled("derive-forgotten-notes", false);
    let parent = other.session(FIRST);
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    other.configure(&only_ids(&provider.url(), &[parent], SUMMARY_ON));
    let made = other.run(&["derive", &parent.to_string()]);
    assert_eq!(made.code, 0, "{:?}", made.stderr);
    assert_eq!(summary_records(&other, parent).len(), 1);
    let (notes, _, _) = other.notes_of(parent).expect("notes");
    forget_entry(&other, notes, None).expect("forgotten");
    refreshed(&other);
    let later = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    other.configure(&only_ids(&later.url(), &[parent], SUMMARY_ON));
    let run = other.run(&["derive", &parent.to_string()]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(requests(&later).is_empty(), "{:?}", requests(&later));
}

#[test]
fn a_session_kept_again_by_hand_gets_its_notes_and_its_summary_again() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-way-back", false);
    let first = sandbox.session(FIRST);
    let file = sandbox.path(&format!("sessions/claude/p/{FIRST}.jsonl"));
    let provider = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&only_ids(&provider.url(), &[first], SUMMARY_ON));
    let made = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(made.code, 0, "{:?}", made.stderr);
    let (notes, before, _) = sandbox.notes_of(first).expect("notes");
    assert_eq!(summary_records(&sandbox, first).len(), 1);

    let forgotten = sandbox.run(&["forget", &first.to_string()]);
    assert_eq!(forgotten.code, 0, "{:?}", forgotten.stderr);
    let quiet = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&only_ids(&quiet.url(), &[first], SUMMARY_ON));
    let run = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(requests(&quiet).is_empty(), "{:?}", requests(&quiet));

    let back = sandbox.run(&["remember", &file]);
    assert_eq!(back.code, 0, "{:?}", back.stderr);
    assert!(
        back.stderr
            .iter()
            .any(|line| line.starts_with("bilbo: kept ") && line.contains(" again: its entry ")),
        "{:?}",
        back.stderr
    );
    let again = Provider::start(vec![answer(NOTE_TEXT), answer(SUMMARY_TEXT)]);
    sandbox.configure(&only_ids(&again.url(), &[first], SUMMARY_ON));
    let run = sandbox.run(&["derive", &first.to_string()]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(requests(&again).iter().any(mentions_the_first));
    let (now, generation, _) = sandbox.notes_of(first).expect("notes again");
    assert_eq!(now, notes);
    assert!(generation > before, "{generation:?} {before:?}");
    let read = sandbox.run(&["read", &first.to_string()]);
    assert!(
        read.stdout.lines().any(|line| line.starts_with("summary:")),
        "{}",
        read.stdout
    );
}

#[test]
fn the_dry_run_says_of_forgotten_entries_what_its_gates_reach() {
    let _alone = alone();
    let sandbox = Sandbox::filled("derive-forgotten-dry", false);
    let first = sandbox.session(FIRST);
    let second = sandbox.session(SECOND);
    let provider = Provider::start(answers(2));
    sandbox.configure(&only_ids(&provider.url(), &[first, second], ""));
    let made = sandbox.run(&["derive"]);
    assert_eq!(made.code, 0, "{:?}", made.stderr);
    let (first_notes, _, _) = sandbox.notes_of(first).expect("notes");
    let (second_notes, _, _) = sandbox.notes_of(second).expect("notes");

    forget_entry(&sandbox, first, None).expect("forgotten");

    let frames = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&frames);
    let hook: bilbo_store::segments::Hook = Box::new(move |step| {
        if step == bilbo_store::segments::Step::FrameWritten
            && counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1
        {
            Err(io::Error::other("disk full"))
        } else {
            Ok(())
        }
    });
    assert!(forget_entry(&sandbox, second, Some(hook)).is_err());

    let kept = sandbox::run(
        sandbox.vars(),
        &["remember", "-"],
        b"A garden note with no file.\n",
        None,
    );
    assert_eq!(kept.code, 0, "{:?}", kept.stderr);
    assert!(kept.stdout.starts_with("new 1, "), "{}", kept.stdout);
    assert!(kept.notices.is_empty(), "{:?}", kept.notices);
    let note_id = Id::parse(
        kept.stderr
            .iter()
            .find_map(|line| line.strip_prefix("bilbo: kept the note as "))
            .expect("the kept line"),
    )
    .expect("an id");
    forget_entry(&sandbox, note_id, None).expect("forgotten");
    refreshed(&sandbox);

    let ids = [first, first_notes, second, second_notes, note_id];
    sandbox.configure(&only_ids(&provider.url(), &ids, SUMMARY_ON));
    let args: Vec<String> = ids.iter().map(ToString::to_string).collect();
    let mut argv = vec!["derive", "--dry-run"];
    argv.extend(args.iter().map(String::as_str));
    let run = sandbox.run(&argv);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let entries: Vec<&str> = dry_lines(&run, "entry\t");
    assert_eq!(
        entries,
        [
            format!("entry\t{first}\tcovered\t-\t0"),
            format!("entry\t{first_notes}\tleft out: no instruction\t-\t0"),
            format!("entry\t{second}\tcovered\t{second_notes}\t1"),
            format!("entry\t{second_notes}\tleft out: no instruction\t-\t0"),
            format!("entry\t{note_id}\tleft out: no instruction\t-\t0"),
        ],
        "{}",
        run.stdout
    );
    let summaries: Vec<&str> = dry_lines(&run, "summary\t");
    assert_eq!(
        summaries,
        [
            format!("summary\t{first}\tnot due: forgotten"),
            format!("summary\t{first_notes}\tnot due: left out: belongs to another entry"),
            format!("summary\t{second}\tnot due: forgotten"),
            format!("summary\t{second_notes}\tnot due: left out: belongs to another entry"),
            format!("summary\t{note_id}\tnot due: forgotten"),
        ],
        "{}",
        run.stdout
    );
    let live = sandbox.run(&["derive", &note_id.to_string()]);
    assert_eq!(live.code, 0, "{:?}", live.stderr);
    assert_eq!(live.stderr, Vec::<String>::new());
}
