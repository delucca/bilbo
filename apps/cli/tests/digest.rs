use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::{Env, Who};
use bilbo_cli::console::indexing::Indexing;
use bilbo_cli::console::private::{self, How};
use bilbo_cli::console::reading::Reading;
use bilbo_cli::console::settings::{self, Settings};
use bilbo_cli::console::view::View;
use bilbo_cli::digest::Hand;
use bilbo_cli::remember::disk::Disk;
use bilbo_entries::remember::{self, Request, Tagging};
use bilbo_search::analyze::Detect;
use bilbo_search::index::{Doc, Extent, Index, IndexError, Stats};
use bilbo_search::lookup::{Edits, Filter, Found, Lookup, Near, Posting, Totals};
use bilbo_search::position::Position;
use bilbo_search::rank::{Query, Ranker};
use bilbo_store::id::Id;
use bilbo_store::segments::{Options, Store};

mod note_line;
mod notes;
mod runner;
mod sandbox;
mod scratch;
mod summaries;

use runner::{Run, Sandbox};

const CLAUDE: &str = "8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b";
const CLAUDE_BYTES: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
);
const ROLLOUT: &str = "rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl";
const ROLLOUT_BYTES: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
);
const SECTIONS: &[u8] = include_bytes!("../../../libs/entries/rust/fixtures/markdown/sections.md");
const SHED: &str = "---\nid: 01JC5Q3V7XKM4P8R2T6W9YB0DH\n---\n# The shed roof\n\nThe shed holds a hoe and the carrots seed. Purslain grows near it.\n";
const PROMPT: &str = "carrots north";

fn escaped(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

fn hook(session: &str, prompt: &str) -> Vec<u8> {
    format!(
        "{{\"session_id\":\"{}\",\"prompt\":\"{}\"}}",
        escaped(session),
        escaped(prompt)
    )
    .into_bytes()
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn fresh() -> String {
    let next = NEXT.fetch_add(1, Ordering::SeqCst);
    format!("00000000-0000-4000-8000-{next:012}")
}

fn failing_index(cache: &Path) -> Box<dyn Index> {
    Box::new(Failing {
        inner: bilbo_cli::index(cache, &bilbo_search::keys::defaults()),
    })
}

fn tune_failing(mut indexing: Indexing<'_>) -> Indexing<'_> {
    indexing.tokenizer = &runner::shared_tokenizer;
    indexing.index = &failing_index;
    indexing
}

fn tune(mut indexing: Indexing<'_>) -> Indexing<'_> {
    indexing.tokenizer = &runner::shared_tokenizer;
    indexing
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = scratch::scratch_dir(test);
        for folder in ["home", "config", "notes"] {
            fs::create_dir_all(dir.join(folder)).expect("create scratch");
        }
        fs::write(dir.join("config/config.toml"), "").expect("an empty config file");
        Sandbox { dir }
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
        self.vars_with(&[])
    }

    fn vars_with(&self, extra: &[(&str, &str)]) -> HashMap<String, OsString> {
        let mut vars = scratch::vars(&self.dir);
        for (name, value) in [
            ("BILBO_SEARCH_WORKERS", "2"),
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
        ] {
            vars.insert(name.to_owned(), OsString::from(value));
        }
        vars.extend(
            extra
                .iter()
                .map(|(name, value)| ((*name).to_owned(), OsString::from(*value))),
        );
        vars
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

    fn keep(&self, args: &[&str]) -> Run {
        let vars = self.vars();
        runner::bilbo_with(
            std::rc::Rc::new(move |name| vars.get(name).cloned()),
            args,
            &tune,
        )
    }

    fn remember(&self, args: &[&str]) {
        let run = self.keep(args);
        assert_eq!(
            (run.code, run.stderr),
            (0, Vec::<String>::new()),
            "{args:?}"
        );
    }

    fn files(&self) {
        self.write("notes/sections.md", SECTIONS);
        self.write("notes/shed.md", SHED.as_bytes());
        self.remember(&["remember", &self.path("notes")]);
    }

    fn sessions(&self) {
        self.write(
            &format!("home/.claude/projects/-home-me-src-garden/{CLAUDE}.jsonl"),
            CLAUDE_BYTES,
        );
        self.write(
            &format!("home/.codex/sessions/2026/11/06/{ROLLOUT}"),
            ROLLOUT_BYTES,
        );
        self.remember(&[
            "remember",
            &self.path("home/.claude/projects"),
            "--kind",
            "session",
        ]);
        self.remember(&[
            "remember",
            &self.path("home/.codex/sessions"),
            "--kind",
            "session",
        ]);
    }

    fn kept_text(&self, text: &str) {
        let run = sandbox::run(self.vars(), &["remember", "-"], text.as_bytes(), None);
        assert_eq!(run.code, 0);
        assert!(
            run.stderr[0].starts_with("bilbo: kept the note as "),
            "{:?}",
            run.stderr
        );
    }

    fn digest(&self, session: &str, prompt: &str) -> sandbox::Run {
        self.digest_with(session, prompt, &[])
    }

    fn digest_with(&self, session: &str, prompt: &str, extra: &[(&str, &str)]) -> sandbox::Run {
        sandbox::run(
            self.vars_with(extra),
            &["digest"],
            &hook(session, prompt),
            None,
        )
    }

    fn first(&self, prompt: &str) -> sandbox::Run {
        let session = fresh();
        self.digest(&session, prompt)
    }

    fn marks(&self) -> Vec<u8> {
        fs::read(self.dir.join("state/bilbo/digest.sessions")).unwrap_or_default()
    }

    fn log_file(&self) -> PathBuf {
        self.dir.join("state/bilbo/digest.log")
    }

    fn log(&self) -> Vec<Vec<String>> {
        tab_fields(&fs::read(self.log_file()).unwrap_or_default())
    }

    fn state(&self) -> PathBuf {
        self.dir.join("state/bilbo")
    }
}

fn item_is_bilbos(item: &str) {
    let (left, score) = item.split_once('=').expect("an item");
    let score = score.strip_suffix('+').unwrap_or(score);
    assert!(score.parse::<f32>().is_ok(), "{item}");
    let (id, range) = match left.split_once("#r") {
        Some((id, range)) => (id, Some(range)),
        None => (left, None),
    };
    assert!(
        id.len() == 26
            && id
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
        "{item}"
    );
    if let Some(range) = range {
        assert!(
            !range.is_empty() && range.chars().all(|c| c.is_ascii_digit() || c == '-'),
            "{item}"
        );
    }
}

fn only_bilbos_words(line: &[String]) {
    let ends = [
        "listed",
        "nothing",
        "floor",
        "later",
        "input",
        "unmarked",
        "busy",
        "no-index",
        "other-manifest",
        "withheld",
        "failed",
    ];
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    assert_eq!(line[0], "d1");
    assert!(digits(&line[1]) && digits(&line[4]), "{line:?}");
    assert!(ends.contains(&line[2].as_str()), "{line:?}");
    assert!(
        line[3] == "-" || line[3].chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "{line:?}"
    );
    assert!(["current", "behind", "unknown", "-"].contains(&line[5].as_str()));
    assert_eq!((line.len() - 6) % 5, 0, "{line:?}");
    for five in line[6..].chunks(5) {
        assert!(["markdown", "session", "note"].contains(&five[0].as_str()));
        assert!(five[1].parse::<f64>().is_ok(), "{five:?}");
        for items in &five[2..4] {
            if items != "-" {
                items.split(',').for_each(item_is_bilbos);
            }
        }
        if five[4] != "-" {
            for cause in five[4].split(',') {
                let (name, count) = cause.split_once('=').expect("a cause");
                assert!(["gone", "own", "belongs", "part", "old", "shown"].contains(&name));
                assert!(digits(count), "{cause}");
            }
        }
    }
}

fn short(id: &str) -> &str {
    &id[id.len() - 10..]
}

#[test]
fn a_first_prompt_lists_the_notes_and_the_second_prints_nothing() {
    let sandbox = Sandbox::new("digest_first");
    sandbox.files();
    let session = fresh();
    let first = sandbox.digest(&session, PROMPT);
    assert_eq!(
        (first.code, first.stderr.clone()),
        (0, Vec::<String>::new())
    );
    let lines: Vec<&str> = first.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "files: 2 may bear on this prompt, best first; bilbo read <ref> outlines one"
    );
    let garden = file_rows(&first.stdout)
        .into_iter()
        .find(|row| row[0] == short("01JC5Q3V7XKM4P8R2T6W9YB0DG"))
        .expect("the garden log's row");
    let fields = garden;
    assert_eq!(fields.len(), 3);
    assert!(fields[1].starts_with("file:") && fields[1].ends_with("sections.md"));
    assert_eq!(fields[2], "The garden log");
    assert!(!first.stdout.contains("sessions:") && !first.stdout.contains("notes:"));
    assert!(first.notices.is_empty(), "{:?}", first.notices);
    let second = sandbox.digest(&session, PROMPT);
    assert_eq!(
        (second.code, second.stdout, second.stderr, second.notices),
        (0, String::new(), Vec::<String>::new(), Vec::<String>::new())
    );
    let other = sandbox.first(PROMPT);
    assert!(other.stdout.starts_with("files: 2"));

    let blank = fresh();
    let spaces = sandbox.digest(&blank, "  \t\n ");
    assert_eq!(
        (spaces.code, spaces.stdout, spaces.stderr, spaces.notices),
        (0, String::new(), Vec::<String>::new(), Vec::<String>::new())
    );
    assert!(
        tab_fields(&sandbox.marks())
            .iter()
            .any(|mark| mark[2] == blank),
        "the session is marked"
    );
    let next = sandbox.digest(&blank, PROMPT);
    assert_eq!(
        (next.code, next.stdout, next.stderr),
        (0, String::new(), Vec::<String>::new())
    );

    let control = sandbox.first(&format!("{PROMPT} \u{FFFD}"));
    let cut = fresh();
    let half = format!("{{\"session_id\":\"{cut}\",\"prompt\":\"{PROMPT} \\ud83d\"}}");
    let halved = sandbox::run(sandbox.vars(), &["digest"], half.as_bytes(), None);
    assert_eq!(
        (halved.code, halved.stderr.clone()),
        (0, Vec::<String>::new())
    );
    assert_eq!(halved.stdout, control.stdout);
    assert_eq!(halved.stdout, other.stdout, "the rows of the plain prompt");
    assert!(halved.stdout.starts_with("files: 2"));
    assert!(
        tab_fields(&sandbox.marks())
            .iter()
            .any(|mark| mark[2] == cut)
    );
    let again = sandbox.digest(&cut, PROMPT);
    assert_eq!(
        (again.code, again.stdout, again.stderr),
        (0, String::new(), Vec::<String>::new())
    );

    let raw = fresh();
    let loose = format!("{{\"session_id\":\"{raw}\",\"prompt\":\"{PROMPT}\t\n\"}}");
    let refused = sandbox::run(sandbox.vars(), &["digest"], loose.as_bytes(), None);
    assert_eq!((refused.code, refused.stdout.as_str()), (0, ""));
    assert_eq!(
        refused.stderr,
        ["bilbo: the hook's input is not a JSON object"]
    );
    assert!(
        !tab_fields(&sandbox.marks())
            .iter()
            .any(|mark| mark[2] == raw),
        "no mark for an input that is no JSON"
    );
    let valid = sandbox.digest(&raw, PROMPT);
    assert_eq!(valid.stdout, other.stdout);
}

const SUBAGENT: &[u8] =
    include_bytes!("../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.jsonl");
const META: &[u8] =
    include_bytes!("../../../libs/entries/rust/fixtures/session/agent-ad63c2b3c9e7a4b3d.meta.json");
const MAIN_BYTES: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
);
const MAIN: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
const KEPT: &str = "The carrots in the north bed need thinning on Monday.";
const BUSY: &str = "bilbo: another bilbo is writing the store";

fn lines(text: &[&str]) -> Vec<String> {
    text.iter().map(|line| (*line).to_owned()).collect()
}

fn tab_fields(bytes: &[u8]) -> Vec<Vec<String>> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

impl Sandbox {
    fn projects(&self) {
        let project = "home/.claude/projects/-home-me-src-garden";
        sandbox::write(
            &self.dir.join(format!("{project}/{MAIN}.jsonl")),
            MAIN_BYTES,
        );
        let child = self.dir.join(format!(
            "{project}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl"
        ));
        sandbox::write(&child, SUBAGENT);
        sandbox::write(&child.with_extension("meta.json"), META);
        self.remember(&[
            "remember",
            &self.path("home/.claude/projects"),
            "--kind",
            "session",
        ]);
    }

    fn entry_of(&self, reference: &str) -> String {
        let read = self.keep(&["read", reference, "--budget", "256"]);
        assert_eq!(read.code, 0, "{reference}: {:?}", read.stderr);
        read.stdout
            .lines()
            .find_map(|line| line.strip_prefix("bilbo:"))
            .expect("an id line")
            .to_owned()
    }

    fn held(&self) -> Store {
        Store::open(&self.settings().layout(), Options::default(), &mut |_| {})
            .expect("open the store")
    }

    fn mark_file(&self) -> PathBuf {
        self.state().join("digest.sessions")
    }
}

fn full(test: &str) -> Sandbox {
    let sandbox = Sandbox::new(test);
    sandbox.files();
    sandbox.sessions();
    sandbox.kept_text(KEPT);
    sandbox
}

fn blocks(stdout: &str) -> Vec<(String, Vec<String>)> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for line in stdout.lines() {
        let word = line.split_once(": ").and_then(|(head, rest)| {
            (!head.contains(['\t', ' ']) && rest.contains("may bear on this prompt"))
                .then_some(head)
        });
        match word {
            Some(word) => found.push((word.to_owned(), Vec::new())),
            None => found
                .last_mut()
                .expect("a first line")
                .1
                .push(line.to_owned()),
        }
    }
    found
}

#[test]
fn a_file_opened_as_a_ledger_is_private_read_from_its_start_and_written_at_its_end() {
    let dir = scratch::scratch_dir("digest_ledger");
    fs::create_dir_all(&dir).expect("a folder");
    let folder = fs::File::open(&dir).expect("an open folder");
    let said = |name: &str| dir.join(name);
    let made = private::open(&folder, "ledger", &said("ledger"), How::Ledger).expect("a new file");
    assert_eq!(
        made.metadata().expect("meta").permissions().mode() & 0o777,
        0o600
    );
    drop(made);
    let mut file = private::open(&folder, "ledger", &said("ledger"), How::Ledger).expect("a file");
    private::append(&mut file, "one\ntwo\n", "the file").expect("an append");
    drop(file);
    let mut file = private::open(&folder, "ledger", &said("ledger"), How::Ledger).expect("a file");
    let mut head = [0_u8; 4];
    file.read_exact(&mut head).expect("a read from the start");
    assert_eq!(&head, b"one\n");
    private::append(&mut file, "three\n", "the file").expect("an append after a read");
    let mut rest = String::new();
    file.read_to_string(&mut rest).expect("the rest");
    assert_eq!(rest, "");
    assert_eq!(
        fs::read_to_string(said("ledger")).expect("the file"),
        "one\ntwo\nthree\n"
    );

    symlink(said("ledger"), said("link")).expect("a link");
    fs::hard_link(said("ledger"), said("twin")).expect("a second name");
    fs::write(said("open"), "x").expect("a file");
    fs::set_permissions(said("open"), fs::Permissions::from_mode(0o644)).expect("a mode");
    for name in ["link", "twin", "ledger", "open"] {
        for how in [How::Ledger, How::Append] {
            let error = private::open(&folder, name, &said(name), how).expect_err(name);
            assert_eq!(
                error.to_string(),
                format!(
                    "{} is not a private file of this user in a folder of this user",
                    said(name).display()
                ),
                "{name} {how:?}"
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn every_path_exits_0_and_says_what_went_wrong() {
    let hello = |session: &str| hook(session, "carrots north");
    let sandbox = Sandbox::new("digest_paths");
    let none = Vec::<String>::new();

    let run = |sandbox: &Sandbox, args: &[&str], input: &[u8]| {
        let run = sandbox::run(sandbox.vars(), args, input, None);
        (run.code, run.stdout, run.stderr)
    };
    let (code, stdout, stderr) = run(&sandbox, &["digest"], &hello("s"));
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        [format!("bilbo: no store at {}", sandbox.path("store"))]
    );

    sandbox.files();
    for (input, said) in [
        (
            b"not json".to_vec(),
            "bilbo: the hook's input is not a JSON object",
        ),
        (
            br#"{"prompt":"p"}"#.to_vec(),
            "bilbo: the hook's input has no session_id",
        ),
        (
            br#"{"session_id":"s"}"#.to_vec(),
            "bilbo: the hook's input has no prompt",
        ),
    ] {
        let (code, stdout, stderr) = run(&sandbox, &["digest"], &input);
        assert_eq!(
            (code, stdout.as_str(), stderr),
            (0, "", lines(&[said])),
            "{said}"
        );
    }
    let long_prompt = "carrots north ".repeat(100);
    let kept_session = fresh();
    let big = hook(&kept_session, &long_prompt);
    assert!(big.len() > 1024 && big.len() < 2048);
    let capped = sandbox::run(
        sandbox.vars_with(&[("BILBO_DIGEST_INPUT", "1 KiB")]),
        &["digest"],
        &big,
        None,
    );
    assert_eq!((capped.code, capped.stdout.as_str()), (0, ""));
    assert_eq!(
        capped.stderr,
        lines(&["bilbo: the hook's input is longer than digest.input allows (1024 bytes)"])
    );
    assert!(
        !tab_fields(&fs::read(sandbox.mark_file()).unwrap_or_default())
            .iter()
            .any(|mark| mark[2] == kept_session),
        "a refused input marks no session"
    );
    let roomy = sandbox::run(
        sandbox.vars_with(&[("BILBO_DIGEST_INPUT", "2 KiB")]),
        &["digest"],
        &big,
        None,
    );
    assert_eq!(roomy.code, 0);
    assert!(roomy.stdout.starts_with("files:"), "{}", roomy.stdout);
    assert!(
        !roomy.stderr.iter().any(|line| line.contains("longer than")),
        "{:?}",
        roomy.stderr
    );
    let (code, stdout, stderr) = run(&sandbox, &["digest", "--all"], b"");
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        lines(&[
            "bilbo: unknown option '--all'",
            "bilbo: usage: bilbo digest [--agy]",
            "bilbo: see 'bilbo digest --help'",
        ])
    );
    let (code, stdout, stderr) = run(&sandbox, &["digest"], &hook(&fresh(), "zzqqxx"));
    assert_eq!((code, stdout.as_str(), stderr), (0, "", none.clone()));

    fs::remove_dir_all(sandbox.dir.join("cache/bilbo/index")).expect("remove the index");
    let (code, stdout, stderr) = run(&sandbox, &["digest"], &hello(&fresh()));
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        lines(&["bilbo: the store has no search index bilbo can use; bilbo reindex makes it"])
    );

    fs::remove_dir_all(sandbox.dir.join("state")).expect("remove the state folder");
    let (code, stdout, stderr) = run(&sandbox, &["digest"], &hello(&fresh()));
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        lines(&["bilbo: this store has no state folder that bilbo checked"])
    );

    sandbox.write("config/config.toml", b"digest.nonsense = 1\n");
    let recall = sandbox::run(sandbox.vars(), &["recall", "carrots"], b"", None);
    assert_eq!(recall.code, 2);
    let (code, stdout, stderr) = run(&sandbox, &["digest"], &hello(&fresh()));
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(stderr, recall.stderr);
}

fn rank_notes(sandbox: &Sandbox, text: &str) -> Vec<(String, f32)> {
    let keys = bilbo_search::recall::keys::defaults();
    let index = bilbo_cli::lookup(
        &sandbox.dir.join("cache/bilbo"),
        &bilbo_search::keys::defaults(),
    );
    let ranked = keys
        .bm25(index.as_ref(), bilbo_cli::detect())
        .rank(
            &Query {
                text: text.to_owned(),
                kinds: Vec::new(),
                apart: Vec::new(),
                entries: None,
            },
            1000,
        )
        .expect("a ranking")
        .candidates;
    let mut seen = Vec::new();
    let mut best = Vec::new();
    for candidate in ranked {
        if !seen.contains(&candidate.found.entry) {
            seen.push(candidate.found.entry);
            best.push((candidate.found.entry.to_string(), candidate.score));
        }
    }
    best
}

fn file_rows(stdout: &str) -> Vec<Vec<String>> {
    blocks(stdout)
        .into_iter()
        .find(|(word, _)| word == "files")
        .map(|(_, rows)| {
            rows.iter()
                .map(|row| row.split('\t').map(str::to_owned).collect())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_notes_rows_and_scores_are_the_same_on_a_store_that_also_holds_sessions() {
    let alone = Sandbox::new("digest_alone");
    alone.files();
    let both = Sandbox::new("digest_both");
    both.remember(&["remember", &alone.path("notes")]);
    both.sessions();
    for prompt in ["north bed carrots", "carrotts north", "purslane hoee"] {
        let oracle = rank_notes(&alone, prompt);
        let mut held: Vec<&str> = oracle.iter().map(|(id, _)| id.as_str()).collect();
        held.sort_unstable();
        assert_eq!(
            held,
            ["01JC5Q3V7XKM4P8R2T6W9YB0DG", "01JC5Q3V7XKM4P8R2T6W9YB0DH"],
            "{prompt}: the files the fixture holds for it"
        );
        let run = both.first(prompt);
        assert_eq!(
            (run.code, run.stderr.clone()),
            (0, Vec::<String>::new()),
            "{prompt}"
        );
        let rows = file_rows(&run.stdout);
        let listed: Vec<String> = rows.iter().map(|row| row[0].clone()).collect();
        let wanted: Vec<String> = oracle
            .iter()
            .map(|(id, _)| id[id.len() - 10..].to_owned())
            .collect();
        assert_eq!(listed, wanted, "{prompt}");
        for ((id, _), row) in oracle.iter().zip(&rows) {
            let file = match id.as_str() {
                "01JC5Q3V7XKM4P8R2T6W9YB0DG" => "sections.md",
                "01JC5Q3V7XKM4P8R2T6W9YB0DH" => "shed.md",
                other => panic!("{other} is no file of the fixture"),
            };
            assert_eq!(
                row[1],
                format!("file:{}", alone.path(&format!("notes/{file}"))),
                "{prompt}"
            );
        }
        let mut distinct: Vec<f32> = oracle.iter().map(|(_, score)| *score).collect();
        distinct.dedup();
        for score in distinct {
            let at = f64::from(score);
            let floor = at.to_string();
            let run = both.digest_with(
                &fresh(),
                prompt,
                &[("BILBO_DIGEST_MARKDOWN_FLOOR", floor.as_str())],
            );
            let wanted: Vec<String> = oracle
                .iter()
                .filter(|(_, held)| f64::from(*held) >= at)
                .map(|(id, _)| id[id.len() - 10..].to_owned())
                .collect();
            let listed: Vec<String> = file_rows(&run.stdout)
                .iter()
                .map(|row| row[0].clone())
                .collect();
            assert_eq!(listed, wanted, "{prompt} at {at}");
            let over = at.next_up().to_string();
            let run = both.digest_with(
                &fresh(),
                prompt,
                &[("BILBO_DIGEST_MARKDOWN_FLOOR", over.as_str())],
            );
            let wanted: Vec<String> = oracle
                .iter()
                .filter(|(_, held)| f64::from(*held) > at)
                .map(|(id, _)| id[id.len() - 10..].to_owned())
                .collect();
            let listed: Vec<String> = file_rows(&run.stdout)
                .iter()
                .map(|row| row[0].clone())
                .collect();
            assert_eq!(listed, wanted, "{prompt} over {at}");
        }
    }
}

#[test]
fn a_note_under_the_floor_is_not_listed() {
    let sandbox = Sandbox::new("digest_floor");
    sandbox.files();
    let held = sandbox.digest_with(&fresh(), PROMPT, &[("BILBO_DIGEST_MARKDOWN_FLOOR", "1000")]);
    assert_eq!(
        (held.code, held.stdout, held.stderr),
        (0, String::new(), Vec::<String>::new())
    );
    let listed = sandbox.digest_with(&fresh(), PROMPT, &[("BILBO_DIGEST_MARKDOWN_FLOOR", "0")]);
    assert!(listed.stdout.starts_with("files: 2"), "{}", listed.stdout);
    let log = sandbox.log();
    assert_eq!(log.len(), 2);
    assert_eq!((log[0][2].as_str(), log[0][8].as_str()), ("floor", "-"));
    assert!(
        log[0][9].contains("01JC5Q3V7XKM4P8R2T6W9YB0DG="),
        "{:?}",
        log[0]
    );
    assert_eq!((log[1][2].as_str(), log[1][9].as_str()), ("listed", "-"));
    assert!(
        log[1][8].contains("01JC5Q3V7XKM4P8R2T6W9YB0DG="),
        "{:?}",
        log[1]
    );
}

struct RefusingTotals {
    inner: Box<dyn Lookup>,
    failed: std::cell::Cell<bool>,
}

impl RefusingTotals {
    fn asked(&self) {
        assert!(
            !self.failed.get(),
            "the index was asked again after its error"
        );
    }
}

impl Lookup for RefusingTotals {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.asked();
        self.inner.position()
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.asked();
        self.failed.set(true);
        Err(IndexError::Failed {
            action: "read",
            path: PathBuf::from("/refused"),
            error: "refused".to_owned(),
        })
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        self.asked();
        self.inner.postings(term)
    }

    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
        self.asked();
        self.inner.near(term, edits)
    }

    fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
        self.asked();
        self.inner.pairs(first, second)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.asked();
        self.inner.holding(filter)
    }

    fn length(&self, kind: &str) -> Result<u64, IndexError> {
        self.asked();
        self.inner.length(kind)
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.asked();
        self.inner.found(docs)
    }
}

struct Failing {
    inner: Box<dyn Index>,
}

impl Index for Failing {
    fn position(&mut self) -> Result<Option<Position>, IndexError> {
        self.inner.position()
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.inner.clear()
    }

    fn add(&mut self, _: &Doc<'_>) -> Result<(), IndexError> {
        self.inner.rollback();
        Err(IndexError::Failed {
            action: "add",
            path: PathBuf::from("/refused"),
            error: "refused".to_owned(),
        })
    }

    fn commit(&mut self, position: &Position) -> Result<Stats, IndexError> {
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

fn direct(
    sandbox: &Sandbox,
    session: &str,
    loaded: &AtomicUsize,
    made: &AtomicUsize,
    extra: &[(&str, &str)],
) -> sandbox::Run {
    direct_through(sandbox, session, loaded, made, extra, false)
}

fn direct_through(
    sandbox: &Sandbox,
    session: &str,
    loaded: &AtomicUsize,
    made: &AtomicUsize,
    extra: &[(&str, &str)],
    refusing: bool,
) -> sandbox::Run {
    let vars = sandbox.vars_with(extra);
    let settings = sandbox.settings_with(extra);
    let readers = bilbo_cli::readers(&settings).expect("the readers");
    let rendering = bilbo_cli::rendering(&settings).expect("the rendering");
    let tokenizer = || {
        loaded.fetch_add(1, Ordering::SeqCst);
        bilbo_cli::tokenizer()
    };
    let lookup = |cache: &Path| {
        made.fetch_add(1, Ordering::SeqCst);
        let real = bilbo_cli::lookup(cache, &settings.search);
        if refusing {
            Box::new(RefusingTotals {
                inner: real,
                failed: std::cell::Cell::new(false),
            })
        } else {
            real
        }
    };
    let apart = bilbo_cli::apart();
    let detect: &dyn Detect = bilbo_cli::detect();
    let reading = Reading {
        readers: &readers,
        chunk: settings.search.chunk(),
        render: &rendering,
        tokenizer: &tokenizer,
        lookup: &lookup,
        detect,
        apart: &apart,
    };
    let width = settings.digest.note_width.value;
    let ways = bilbo_cli::note_ways(width);
    let (shelves, way) = bilbo_cli::hands(&settings, &readers, &ways, None).expect("the hands");
    let hand = Hand {
        tokenizer: runner::tokenizer().id,
        shelves: &shelves,
        way,
    };
    let said = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&said);
    let mut env = Env {
        var: std::rc::Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::Cursor::new(hook(session, PROMPT))),
        who: Who::Nobody,
        notice: Box::new(move |message| {
            if let Ok(mut notices) = sink.lock() {
                notices.extend(message.plain_lines());
            }
        }),
    };
    let (code, stdout, stderr) = match bilbo_cli::digest::run(
        &bilbo_cli::bilbo(),
        &bilbo_cli::digest::Args { agy: false },
        &mut env,
        &settings,
        &reading,
        &hand,
    ) {
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
    let notices = said.lock().map(|held| held.clone()).unwrap_or_default();
    sandbox::Run {
        code,
        stdout,
        stderr,
        notices,
    }
}

#[test]
fn a_later_prompt_opens_no_store_and_loads_no_tokenizer() {
    let sandbox = Sandbox::new("digest_later");
    sandbox.files();
    let session = fresh();
    let first = sandbox.digest(&session, PROMPT);
    assert!(first.stdout.starts_with("files:"));
    let (loaded, made) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let later = direct(&sandbox, &session, &loaded, &made, &[]);
    assert_eq!(
        (later.code, later.stdout, later.stderr, later.notices),
        (0, String::new(), Vec::<String>::new(), Vec::<String>::new())
    );
    assert_eq!(
        (loaded.load(Ordering::SeqCst), made.load(Ordering::SeqCst)),
        (0, 0)
    );
    let new = direct(&sandbox, &fresh(), &loaded, &made, &[]);
    assert!(new.stdout.starts_with("files: 2"), "{:?}", new.stderr);
    assert!(new.notices.is_empty(), "{:?}", new.notices);
    assert_eq!(
        (loaded.load(Ordering::SeqCst), made.load(Ordering::SeqCst)),
        (0, 1)
    );

    let store = sandbox.held();
    let held = sandbox.digest_with(&session, PROMPT, &[("BILBO_DIGEST_WAIT", "0s")]);
    assert_eq!(
        (held.code, held.stdout, held.stderr, held.notices),
        (0, String::new(), Vec::<String>::new(), Vec::<String>::new())
    );
    drop(store);
    fs::remove_dir_all(sandbox.dir.join("cache")).expect("remove the cache folder");
    let gone = sandbox.digest(&session, PROMPT);
    assert_eq!(
        (gone.code, gone.stdout, gone.stderr, gone.notices),
        (0, String::new(), Vec::<String>::new(), Vec::<String>::new())
    );
}

#[test]
fn a_first_prompt_that_finds_the_store_held_is_marked() {
    let sandbox = Sandbox::new("digest_busy");
    sandbox.files();
    let busy = fresh();
    let store = sandbox.held();
    let held = sandbox.digest_with(&busy, PROMPT, &[("BILBO_DIGEST_WAIT", "0s")]);
    assert_eq!((held.code, held.stdout.as_str()), (0, ""));
    assert_eq!(held.stderr, lines(&[BUSY]));
    assert!(held.notices.is_empty(), "{:?}", held.notices);
    let marks = tab_fields(&sandbox.marks());
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0][2], busy);
    drop(store);
    let other = sandbox.first(PROMPT);
    assert!(other.stdout.starts_with("files: 2"), "{:?}", other.stderr);
    assert!(
        file_rows(&other.stdout)
            .iter()
            .any(|row| row[0] == short("01JC5Q3V7XKM4P8R2T6W9YB0DG")),
        "{}",
        other.stdout
    );
    let again = sandbox.digest(&busy, PROMPT);
    assert_eq!(
        (again.code, again.stdout, again.stderr, again.notices),
        (0, String::new(), Vec::<String>::new(), Vec::<String>::new())
    );
}

#[test]
fn a_first_prompt_that_waits_for_a_writer_tells_the_user_nothing() {
    let sandbox = Sandbox::new("digest_waits");
    sandbox.files();
    let session = fresh();
    let store = sandbox.held();
    let run = sandbox::run(
        sandbox.vars_with(&[("BILBO_DIGEST_WAIT", "100ms")]),
        &["digest"],
        &hook(&session, PROMPT),
        None,
    );
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(run.stderr, lines(&[BUSY]));
    assert!(run.notices.is_empty(), "{:?}", run.notices);
    drop(store);
    assert_eq!(tab_fields(&sandbox.marks())[0][2], session);
}

#[test]
fn the_mark_is_a_private_file() {
    let sandbox = Sandbox::new("digest_private");
    sandbox.files();
    let session = fresh();
    let run = sandbox.digest(&session, PROMPT);
    assert!(run.stdout.starts_with("files:"));
    let file = sandbox.mark_file();
    let meta = fs::metadata(&file).expect("the marks");
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let marks = tab_fields(&sandbox.marks());
    assert_eq!(marks.len(), 1);
    assert_eq!(
        (marks[0].len(), marks[0][0].as_str(), marks[0][2].as_str()),
        (3, "s1", session.as_str())
    );
    let target = sandbox.dir.join("elsewhere");
    for how in ["link", "twin", "open"] {
        fs::remove_file(&file).expect("remove the marks");
        fs::write(&target, "kept\n").expect("a target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("a mode");
        match how {
            "link" => symlink(&target, &file).expect("a link"),
            "twin" => fs::hard_link(&target, &file).expect("a second name"),
            _ => {
                fs::write(&file, "").expect("a file");
                fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("a mode");
            }
        }
        let run = sandbox.first(PROMPT);
        assert_eq!((run.code, run.stdout.as_str()), (0, ""), "{how}");
        assert_eq!(run.stderr.len(), 1, "{how}: {:?}", run.stderr);
        assert_eq!(
            run.stderr[0],
            format!(
                "bilbo: the session was not marked, so nothing was listed: {} is not a private file of this user in a folder of this user",
                file.display()
            ),
            "{how}"
        );
        assert_eq!(
            fs::read_to_string(&target).expect("the target"),
            "kept\n",
            "{how}"
        );
        assert_eq!(
            fs::read(&file).map_or(0, |bytes| bytes.len()),
            if how == "open" { 0 } else { 5 },
            "{how}"
        );
    }
}

#[test]
fn a_session_that_cannot_be_marked_gets_nothing_twice() {
    let sandbox = Sandbox::new("digest_unmarked");
    sandbox.files();
    fs::create_dir(sandbox.mark_file()).expect("a folder in place of the marks");
    let session = fresh();
    for _ in 0..2 {
        let run = sandbox.digest(&session, PROMPT);
        assert_eq!((run.code, run.stdout.as_str()), (0, ""));
        assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
        assert!(
            run.stderr[0].starts_with("bilbo: the session was not marked, so nothing was listed: "),
            "{:?}",
            run.stderr
        );
    }
}

#[test]
fn a_torn_mark_never_marks_a_shorter_session() {
    let sandbox = Sandbox::new("digest_torn");
    sandbox.files();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(sandbox.mark_file())
        .expect("the marks");
    io::Write::write_all(&mut file, b"s1\t5\tfirst\ns1\t6\tgarden").expect("a torn mark");
    drop(file);
    let shed = sandbox.digest("shed", PROMPT);
    assert!(shed.stdout.starts_with("files: 2"), "{:?}", shed.stderr);
    let marks = tab_fields(&sandbox.marks());
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0], ["s1", "5", "first"]);
    assert_eq!((marks[1][0].as_str(), marks[1][2].as_str()), ("s1", "shed"));
    let bytes = sandbox.marks();
    assert!(bytes.ends_with(b"\tshed\n"));
    assert!(!String::from_utf8_lossy(&bytes).contains("garden"));
    let garden = sandbox.digest("garden", PROMPT);
    assert!(garden.stdout.starts_with("files: 2"), "{:?}", garden.stderr);
}

#[test]
fn an_index_behind_the_store_still_lists() {
    let sandbox = Sandbox::new("digest_behind");
    sandbox.files();
    sandbox.sessions();
    let path = sandbox.dir.join(format!(
        "home/.claude/projects/-home-me-src-garden/{CLAUDE}.jsonl"
    ));
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
            tags: Tagging::default(),
            run: bilbo_entries::remember::Run::ByHand,
        },
        &kinds,
        &Disk,
    )
    .expect("a plan");
    let mut store =
        Store::open(&settings.layout(), Options::default(), &mut |_| {}).expect("open the store");
    remember::keep(&plan, &Disk, &mut store).expect("keep the growth");
    drop(store);
    let run = sandbox.first(PROMPT);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let listed: Vec<String> = blocks(&run.stdout)
        .into_iter()
        .map(|(word, _)| word)
        .collect();
    assert_eq!(listed, ["files", "sessions"]);
}

struct Terminal(io::Cursor<Vec<u8>>, std::sync::Arc<AtomicUsize>);

impl Read for Terminal {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.read(buffer)
    }
}

#[test]
fn a_person_at_the_terminal_is_told_what_it_reads() {
    let sandbox = Sandbox::new("digest_person");
    sandbox.files();
    let reads = std::sync::Arc::new(AtomicUsize::new(0));
    let run = sandbox::run_as(
        sandbox.vars(),
        &["digest"],
        Who::Person,
        Box::new(Terminal(
            io::Cursor::new(hook("typed", PROMPT)),
            std::sync::Arc::clone(&reads),
        )),
        None,
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0, "nothing was read");
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(
        run.stderr,
        lines(&[
            "bilbo: digest reads a prompt hook's JSON on standard input",
            "bilbo: usage: bilbo digest [--agy]",
            "bilbo: see 'bilbo digest --help'",
        ])
    );
    assert!(!sandbox.mark_file().exists());
}

fn outline_line_at(sandbox: &Sandbox, reference: &str, first: u64) -> String {
    let read = sandbox.keep(&["read", reference, "--outline"]);
    assert_eq!(read.code, 0, "{:?}", read.stderr);
    let mut held = None;
    for line in read.stdout.lines() {
        let Some(rest) = line.strip_prefix("#r") else {
            continue;
        };
        let Some((locator, text)) = rest.split_once(' ') else {
            continue;
        };
        let at: u64 = locator
            .split('-')
            .next()
            .and_then(|n| n.parse().ok())
            .expect("a record");
        if at <= first {
            held = Some(text.to_owned());
        }
    }
    held.expect("an outline line")
}

#[test]
fn a_first_prompt_lists_every_kind_each_under_its_line() {
    let sandbox = full("digest_every");
    let run = sandbox.first(PROMPT);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let found = blocks(&run.stdout);
    let words: Vec<&str> = found.iter().map(|(word, _)| word.as_str()).collect();
    assert_eq!(words, ["files", "sessions", "notes"]);
    assert!(run.stdout.lines().all(|line| !line.is_empty()));
    let first_lines: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.contains("may bear"))
        .collect();
    assert_eq!(
        first_lines,
        [
            "files: 2 may bear on this prompt, best first; bilbo read <ref> outlines one",
            "sessions: 1 may bear on this prompt; bilbo read <ref> prints it",
            "notes: 1 may bear on this prompt; bilbo read <ref> prints it",
        ]
    );
    let garden: Vec<&str> = found[0]
        .1
        .iter()
        .find(|row| row.starts_with("2T6W9YB0DG"))
        .expect("the garden log")
        .split('\t')
        .collect();
    assert_eq!(garden.len(), 3);
    assert_eq!(garden[2], "The garden log");
    let session: Vec<&str> = found[1].1[0].split('\t').collect();
    assert_eq!(session.len(), 4);
    let (reference, records) = session[0].split_once("#r").expect("a locator");
    assert_eq!(sandbox.entry_of(reference).len(), 26);
    assert_eq!(records, "3-20", "the whole passage of the fixture");
    let first: u64 = records
        .split('-')
        .next()
        .and_then(|n| n.parse().ok())
        .expect("records");
    assert_eq!(session[1], "2026-11-07");
    assert_eq!(session[2], "/home/me/src/garden");
    assert_eq!(
        session[3],
        outline_line_at(&sandbox, &sandbox.entry_of(reference), first)
    );
    let note: Vec<&str> = found[2].1[0].split('\t').collect();
    assert_eq!(note.len(), 4);
    assert!(note[0].ends_with("#r1"));
    let read = sandbox.keep(&["read", note[0]]);
    assert_eq!(read.code, 0, "{:?}", read.stderr);
    let day = read
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("#r1 "))
        .expect("the day line");
    assert_eq!(note[1], day);
    assert_eq!(note[2], "-");
    assert_eq!(note[3], KEPT);
}

#[test]
fn a_list_with_no_row_is_not_printed_and_a_list_turned_off_changes_no_other() {
    let sandbox = full("digest_off");
    let all = sandbox.first(PROMPT);
    let names = |run: &sandbox::Run| -> Vec<String> {
        blocks(&run.stdout)
            .into_iter()
            .map(|(word, _)| word)
            .collect()
    };
    for (key, wanted) in [
        ("BILBO_DIGEST_MARKDOWN_FLOOR", ["sessions", "notes"]),
        ("BILBO_DIGEST_SESSION_FLOOR", ["files", "notes"]),
        ("BILBO_DIGEST_NOTE_FLOOR", ["files", "sessions"]),
    ] {
        let run = sandbox.digest_with(&fresh(), PROMPT, &[(key, "1000")]);
        assert_eq!(names(&run), wanted, "{key}");
        for (word, rows) in blocks(&run.stdout) {
            let whole = blocks(&all.stdout);
            let same = whole
                .iter()
                .find(|(held, _)| *held == word)
                .expect("a list");
            assert_eq!(rows, same.1, "{key} {word}");
        }
    }
    let off = sandbox.digest_with(&fresh(), PROMPT, &[("BILBO_DIGEST_SESSION_MOST", "0")]);
    let mut wanted = String::new();
    let mut skipping = false;
    for line in all.stdout.lines() {
        if line.contains("may bear on this prompt") {
            skipping = line.starts_with("sessions: ");
        }
        if !skipping {
            wanted.push_str(line);
            wanted.push('\n');
        }
    }
    assert_eq!(off.stdout, wanted);
}

#[test]
fn the_prompts_own_session_is_not_listed() {
    let sandbox = full("digest_own");
    let own = sandbox.digest(CLAUDE, PROMPT);
    assert_eq!((own.code, own.stderr.clone()), (0, Vec::<String>::new()));
    let words: Vec<String> = blocks(&own.stdout)
        .into_iter()
        .map(|(word, _)| word)
        .collect();
    assert_eq!(words, ["files", "notes"]);
    let other = sandbox.first(PROMPT);
    let found = blocks(&other.stdout);
    assert_eq!(found[1].0, "sessions");
    let entry = sandbox.entry_of(CLAUDE);
    assert!(found[1].1[0].starts_with(&entry[16..]));
}

#[test]
fn a_subagents_file_and_an_old_session_are_no_rows() {
    let sandbox = Sandbox::new("digest_child");
    sandbox.files();
    sandbox.projects();
    let prompt = "seed packets shed tomatoes north";
    let run = sandbox.first(prompt);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let found = blocks(&run.stdout);
    let words: Vec<&str> = found.iter().map(|(word, _)| word.as_str()).collect();
    assert_eq!(words, ["files", "sessions"]);
    assert_eq!(found[1].1.len(), 1);
    let main = sandbox.entry_of(MAIN);
    assert!(found[1].1[0].starts_with(&main[16..]), "{:?}", found[1].1);
    let young = sandbox.digest_with(
        &fresh(),
        prompt,
        &[("BILBO_DIGEST_SESSION_OLD_AFTER", "1d")],
    );
    let found_young = blocks(&young.stdout);
    let words: Vec<&str> = found_young.iter().map(|(word, _)| word.as_str()).collect();
    assert_eq!(words, ["files"]);
    assert_eq!(found_young[0].1, found[0].1);
    let with = sandbox.digest_with(
        &fresh(),
        prompt,
        &[("BILBO_DIGEST_SESSION_SUBAGENTS", "true")],
    );
    let found_with = blocks(&with.stdout);
    assert_eq!(found_with[1].0, "sessions");
    assert_eq!(found_with[1].1.len(), 2, "{:?}", found_with[1].1);
    assert!(
        found_with[1]
            .1
            .iter()
            .any(|row| row.starts_with(&main[16..])),
        "{:?}",
        found_with[1].1
    );
}

#[test]
fn a_session_with_notes_written_from_it_is_listed_once_with_its_note() {
    let sandbox = full("digest_note");
    let parent = Id::parse(&sandbox.entry_of(CLAUDE)).expect("an id");
    let layout = sandbox.settings().layout();
    let wanted =
        "Purslane  was identified in the corner of the north bed and it is edible and can stay";
    notes::noted(
        &layout,
        parent,
        &[
            note_line::line(
                parent,
                "The user asked about the seedlings left",
                Some(1_794_039_249),
                &[(3, 4)],
            ),
            note_line::line(parent, wanted, Some(1_794_039_300), &[(3, 20)]),
            note_line::line(
                parent,
                "The shed was checked for a hoe",
                Some(1_794_039_400),
                &[(10, 11)],
            ),
        ],
    );
    let reindexed = sandbox.keep(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{:?}", reindexed.stderr);
    let prompt = "carrots north purslane";
    let squeezed = wanted.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = squeezed
        .chars()
        .take(39)
        .chain(std::iter::once('\u{2026}'))
        .collect();
    let width = [("BILBO_DIGEST_NOTE_WIDTH", "40")];
    let run = sandbox.digest_with(&fresh(), prompt, &width);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let found = blocks(&run.stdout);
    let words: Vec<&str> = found.iter().map(|(word, _)| word.as_str()).collect();
    assert_eq!(words, ["files", "sessions", "notes"]);
    let session: Vec<&str> = found[1].1[0].split('\t').collect();
    assert!(session[0].ends_with("#r3-20"), "{}", session[0]);
    assert_eq!(session.len(), 5);
    assert_eq!(session[4], format!("note: {cut}"));
    assert_eq!(found[2].1.len(), 1);
    assert!(found[2].1[0].contains(&KEPT[..20]), "{:?}", found[2].1);
    for row in &found[2].1 {
        assert!(!row.contains("Purslane"), "{row}");
    }

    let off = sandbox.digest_with(
        &fresh(),
        prompt,
        &[
            ("BILBO_DIGEST_NOTE_WIDTH", "40"),
            ("BILBO_DIGEST_SESSION_MOST", "0"),
        ],
    );
    let found = blocks(&off.stdout);
    let notes = found
        .iter()
        .find(|(word, _)| word == "notes")
        .expect("the notes");
    let derived = notes
        .1
        .iter()
        .map(|row| row.split('\t').collect::<Vec<_>>())
        .find(|fields| fields[3] == cut)
        .expect("a row for the notes entry");
    assert_eq!(derived.len(), 4);
    assert_eq!(derived[1], "2026-11-07");
    assert_eq!(derived[2], "-");
    assert!(derived[0].ends_with("#r2"), "{derived:?}");
    let read = sandbox.keep(&["read", derived[0]]);
    assert_eq!(read.code, 0, "{:?}", read.stderr);
    assert!(read.stdout.contains(wanted), "{}", read.stdout);
    assert!(
        read.stdout
            .contains(&format!("{parent}#g1:r3-20 2026-11-07")),
        "{}",
        read.stdout
    );
}

#[test]
fn the_log_has_one_line_a_run_and_no_word_of_a_prompt() {
    let sandbox = full("digest_log");
    let session = fresh();
    let listed = sandbox.digest(&session, PROMPT);
    assert!(listed.stdout.starts_with("files:"));
    let later = sandbox.digest(&session, PROMPT);
    assert_eq!(later.stdout, "");
    let bad = sandbox::run(sandbox.vars(), &["digest"], b"not json", None);
    assert_eq!(bad.code, 0);
    let named = fresh();
    let nothing_to_ask = sandbox::run(
        sandbox.vars(),
        &["digest"],
        format!("{{\"session_id\":\"{named}\"}}").as_bytes(),
        None,
    );
    assert_eq!(nothing_to_ask.code, 0);
    let nothing = fresh();
    let none = sandbox.digest(&nothing, "zzqqxx");
    assert_eq!(none.stdout, "");
    let log = sandbox.log();
    assert_eq!(log[4][3], nothing);
    let words: Vec<&str> = log.iter().map(|line| line[2].as_str()).collect();
    assert_eq!(words, ["listed", "later", "input", "input", "nothing"]);
    assert_eq!(log[0][3], session);
    assert_eq!(log[1][3], session);
    assert_eq!(log[2][3], "-");
    assert_eq!(log[3][3], named);
    for line in &log {
        assert_eq!(line[0], "d1");
        assert!(line[1].parse::<u64>().is_ok() && line[4].parse::<u64>().is_ok());
    }
    assert_eq!(
        [log[1].len(), log[2].len(), log[3].len()],
        [6, 6, 6],
        "a later line and the two input lines"
    );
    assert_eq!((log[0].len(), log[4].len()), (21, 21));
    assert_eq!(log[0][5], "current");
    assert_eq!(log[4][5], "current");
    assert_eq!(log[0][6], "markdown");
    assert_eq!(log[0][11], "session");
    assert_eq!(log[0][16], "note");
    assert_eq!(log[4][8..11], ["-", "-", "-"]);
    let meta = fs::metadata(sandbox.log_file()).expect("the log");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let text = fs::read_to_string(sandbox.log_file()).expect("the log");
    log.iter().for_each(|line| only_bilbos_words(line));
    for word in [
        "carrots",
        "north",
        "zzqqxx",
        "garden",
        "Beds",
        "Watering",
        "Harvest",
        "hoe",
        "rake",
        "roof",
        "seed",
        "shed",
        "Purslain",
        "seedlings",
        "Thin",
        "carrot",
        "purslane",
        "weed",
        "Water",
        "beans",
        "Monday",
        "thinning",
        "/home/me",
    ] {
        assert!(!text.contains(word), "{word}");
    }
}

#[test]
fn a_log_that_cannot_be_written_changes_no_row() {
    let sandbox = Sandbox::new("digest_unlogged");
    sandbox.files();
    let target = sandbox.dir.join("elsewhere");
    fs::write(&target, "kept\n").expect("a target");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("a mode");
    let control = sandbox.first(PROMPT);
    assert!(control.stdout.starts_with("files: 2"), "{}", control.stdout);
    assert_eq!(control.stderr, Vec::<String>::new());
    fs::remove_file(sandbox.log_file()).expect("the log goes");
    symlink(&target, sandbox.log_file()).expect("a link");
    let run = sandbox.first(PROMPT);
    assert_eq!(run.stdout, control.stdout);
    let unlogged = format!(
        "bilbo: the digest was not logged: {} is not a private file of this user in a folder of this user",
        sandbox.log_file().display()
    );
    assert_eq!(run.stderr, std::slice::from_ref(&unlogged));
    assert_eq!(fs::read_to_string(&target).expect("the target"), "kept\n");

    let store = sandbox.held();
    let vars = sandbox.vars_with(&[("BILBO_DIGEST_WAIT", "0s")]);
    let mut env = Env {
        var: std::rc::Rc::new(move |name| vars.get(name).cloned()),
        stdin: Box::new(io::Cursor::new(hook(&fresh(), PROMPT))),
        who: Who::Nobody,
        notice: Box::new(|_| {}),
    };
    let outcome = bilbo_cli::run(
        &["bilbo".into(), "digest".into()],
        &mut env,
        Ok(sandbox.settings_with(&[("BILBO_DIGEST_WAIT", "0s")])),
    )
    .expect("an outcome");
    drop(store);
    assert_eq!(outcome.stdout(&View::PLAIN), "");
    assert_eq!(
        outcome.stderr_lines(&View::PLAIN),
        lines(&[BUSY, &unlogged])
    );
    assert_eq!(
        (outcome.exit_code(), outcome.write_failure_code()),
        (0, 0),
        "the root gives the exit, also for a busy store and a log that cannot be written"
    );
}

#[test]
fn digest_log_false_writes_no_log() {
    let sandbox = Sandbox::new("digest_nolog");
    sandbox.files();
    let run = sandbox.digest_with(&fresh(), PROMPT, &[("BILBO_DIGEST_LOG_ENABLED", "false")]);
    assert!(run.stdout.starts_with("files: 2"));
    assert_eq!(run.stderr, Vec::<String>::new());
    assert!(!sandbox.log_file().exists());
    let on = sandbox.first(PROMPT);
    assert!(on.stdout.starts_with("files: 2"));
    assert!(sandbox.log_file().exists());
}

#[test]
fn each_way_a_first_prompt_ends_reaches_the_log() {
    let sandbox = Sandbox::new("digest_ways");
    sandbox.files();
    sandbox.sessions();
    let last = |sandbox: &Sandbox| -> Vec<String> { sandbox.log().pop().expect("a line") };
    let third_and_sixth = |line: &[String]| (line[2].clone(), line[5].clone());
    let ends = |word: &str, standing: &str| (word.to_owned(), standing.to_owned());

    let listed = sandbox.first(PROMPT);
    assert!(listed.stdout.starts_with("files:"));
    assert_eq!(third_and_sixth(&last(&sandbox)), ends("listed", "current"));

    let store = sandbox.held();
    let busy = sandbox.digest_with(&fresh(), PROMPT, &[("BILBO_DIGEST_WAIT", "0s")]);
    assert_eq!(busy.stderr, lines(&[BUSY]));
    drop(store);
    let line = last(&sandbox);
    assert_eq!(third_and_sixth(&line), ends("busy", "-"));
    assert_eq!(line.len(), 6);

    let behind_path = sandbox.dir.join(format!(
        "home/.claude/projects/-home-me-src-garden/{CLAUDE}.jsonl"
    ));
    let mut grown = fs::read(&behind_path).expect("the session");
    let record = grown[..grown.len() - 1]
        .split(|byte| *byte == b'\n')
        .rfind(|line| line.starts_with(b"{\"parentUuid\""))
        .expect("a record")
        .to_vec();
    grown.extend_from_slice(&record);
    grown.push(b'\n');
    fs::write(&behind_path, &grown).expect("append to the session");
    let settings = sandbox.settings();
    let kinds = bilbo_cli::kinds(&settings).expect("the kinds");
    let plan = remember::plan(
        &Request {
            target: behind_path,
            kind: None,
            tags: Tagging::default(),
            run: bilbo_entries::remember::Run::ByHand,
        },
        &kinds,
        &Disk,
    )
    .expect("a plan");
    let mut writer =
        Store::open(&settings.layout(), Options::default(), &mut |_| {}).expect("open the store");
    remember::keep(&plan, &Disk, &mut writer).expect("keep the growth");
    drop(writer);
    let behind = sandbox.first(PROMPT);
    assert!(behind.stdout.starts_with("files:"), "{:?}", behind.stderr);
    assert_eq!(third_and_sixth(&last(&sandbox)), ends("listed", "behind"));

    let failed = direct_through(
        &sandbox,
        &fresh(),
        &AtomicUsize::new(0),
        &AtomicUsize::new(0),
        &[],
        true,
    );
    assert_eq!(failed.stdout, "");
    assert_eq!(
        failed.stderr,
        lines(&[
            "bilbo: cannot read the index /refused: refused; bilbo reindex makes the index again"
        ])
    );
    let line = last(&sandbox);
    assert_eq!(third_and_sixth(&line), ends("failed", "-"));
    assert_eq!(line.len(), 6);

    let marks = sandbox.mark_file();
    fs::rename(&marks, sandbox.dir.join("marks-aside")).expect("the marks go aside");
    fs::create_dir(&marks).expect("a folder in place of the marks");
    let unmarked = sandbox.first(PROMPT);
    assert_eq!(unmarked.stdout, "");
    assert_eq!(third_and_sixth(&last(&sandbox)), ends("unmarked", "-"));
    fs::remove_dir(&marks).expect("the folder goes");

    let chunks = sandbox.digest_with(&fresh(), PROMPT, &[("BILBO_SEARCH_CHUNK_TOKENS", "256")]);
    assert_eq!(chunks.stdout, "");
    assert_eq!(
        third_and_sixth(&last(&sandbox)),
        ends("other-manifest", "-")
    );

    let catalog = sandbox.dir.join("cache/bilbo/catalog.sqlite");
    let saved = fs::read(&catalog).expect("the catalog");
    fs::remove_file(&catalog).expect("remove the catalog");
    let withheld = sandbox.first(PROMPT);
    assert_eq!(withheld.stdout, "");
    assert_eq!(third_and_sixth(&last(&sandbox)), ends("withheld", "-"));

    sandbox.write("notes/extra.md", b"# Extra\n\nmore about the garden\n");
    let vars = sandbox.vars();
    let again = runner::bilbo_with(
        std::rc::Rc::new(move |name| vars.get(name).cloned()),
        &["remember", &sandbox.path("notes")],
        &tune_failing,
    );
    assert_eq!(again.code, 1, "{:?}", again.stderr);
    drop(saved);
    let unknown = sandbox.first(PROMPT);
    assert!(unknown.stdout.starts_with("files:"), "{:?}", unknown.stderr);
    assert_eq!(third_and_sixth(&last(&sandbox)), ends("listed", "unknown"));

    fs::remove_dir_all(sandbox.dir.join("cache/bilbo/index")).expect("remove the index");
    let session = fresh();
    let none = sandbox.digest(&session, PROMPT);
    assert_eq!(none.stdout, "");
    let line = last(&sandbox);
    assert_eq!(line.len(), 6);
    assert_eq!(third_and_sixth(&line), ends("no-index", "-"));
    assert_eq!(line[3], session);

    fs::remove_dir_all(sandbox.dir.join("cache")).expect("remove the cache folder");
    let bare = fresh();
    let missing = sandbox.digest(&bare, PROMPT);
    assert_eq!((missing.code, missing.stdout.as_str()), (0, ""));
    assert_eq!(
        missing.stderr,
        lines(&["bilbo: the store has no search index bilbo can use; bilbo reindex makes it"])
    );
    let line = last(&sandbox);
    assert_eq!(line.len(), 6);
    assert_eq!(third_and_sixth(&line), ends("no-index", "-"));
    assert_eq!(line[3], bare);
}

#[test]
fn the_logged_scores_are_the_plain_rankers_over_the_notes_alone() {
    let alone = Sandbox::new("digest_logged_alone");
    alone.files();
    let both = Sandbox::new("digest_logged_both");
    both.remember(&["remember", &alone.path("notes")]);
    both.sessions();
    let read = |text: &str| -> Vec<(String, u32)> {
        if text == "-" {
            return Vec::new();
        }
        text.split(',')
            .map(|item| {
                let (id, score) = item.split_once('=').expect("an item");
                let score: f32 = score.parse().expect("a score");
                (id.to_owned(), score.to_bits())
            })
            .collect()
    };
    for prompt in ["north bed carrots", "carrotts north", "purslane hoee"] {
        let oracle: Vec<(String, u32)> = rank_notes(&alone, prompt)
            .into_iter()
            .map(|(id, score)| (id, score.to_bits()))
            .collect();
        assert!(!oracle.is_empty(), "{prompt}");
        both.first(prompt);
        let line = both.log().pop().expect("a line");
        assert_eq!(read(&line[8]), oracle, "{prompt}: listed");
        both.digest_with(
            &fresh(),
            prompt,
            &[
                ("BILBO_DIGEST_MARKDOWN_FLOOR", "1000"),
                ("BILBO_DIGEST_SESSION_FLOOR", "1000"),
                ("BILBO_DIGEST_NOTE_FLOOR", "1000"),
            ],
        );
        let line = both.log().pop().expect("a line");
        assert_eq!(line[2], "floor", "{prompt}");
        assert_eq!(read(&line[9]), oracle, "{prompt}: held back");
    }
}

#[test]
fn a_torn_log_line_never_takes_the_next_runs_line() {
    let sandbox = Sandbox::new("digest_torn_log");
    sandbox.files();
    let whole = "d1\t1\tlater\tx\t1\t-\n";
    let clean = fresh();
    let control = sandbox.digest(&clean, PROMPT);
    assert!(
        control.stdout.starts_with("files: 2"),
        "{:?}",
        control.stderr
    );
    let masked = |line: &str, session: &str| -> String {
        let mut fields: Vec<String> = line
            .trim_end_matches('\n')
            .split('\t')
            .map(str::to_owned)
            .collect();
        assert_eq!(fields[3], session);
        fields[1] = "-".to_owned();
        fields[3] = "session".to_owned();
        fields[4] = "-".to_owned();
        format!("{}\n", fields.join("\t"))
    };
    let clean_line = masked(
        &fs::read_to_string(sandbox.log_file()).expect("the control's log"),
        &clean,
    );
    for (held, kept) in [
        (format!("{whole}d1\t5\tli"), whole.to_owned()),
        ("d1\t5\tli".to_owned(), String::new()),
    ] {
        let _ = fs::remove_file(sandbox.log_file());
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(sandbox.log_file())
            .expect("the log");
        io::Write::write_all(&mut file, held.as_bytes()).expect("a torn line");
        drop(file);
        let session = fresh();
        let run = sandbox.digest(&session, PROMPT);
        assert!(run.stdout.starts_with("files: 2"), "{:?}", run.stderr);
        assert_eq!(run.stderr, Vec::<String>::new());
        let text = fs::read_to_string(sandbox.log_file()).expect("the log");
        let own = text.strip_prefix(&kept).expect("the whole line kept");
        assert!(own.starts_with("d1\t") && own.ends_with('\n'), "{own:?}");
        assert_eq!(own.matches('\n').count(), 1, "{own:?}");
        let fields: Vec<String> = own.trim_end().split('\t').map(str::to_owned).collect();
        only_bilbos_words(&fields);
        assert_eq!(fields.len(), 21);
        assert_eq!(fields[0], "d1");
        assert_eq!(fields[2], "listed");
        assert_eq!(fields[3], session);
        assert_eq!(fields[5], "current");
        let shown: Vec<String> = file_rows(&run.stdout)
            .iter()
            .map(|row| short(&row[0]).to_owned())
            .collect();
        let listed: Vec<&str> = fields[8].split(',').collect();
        assert_eq!(listed.len(), shown.len());
        for (item, short) in listed.iter().zip(&shown) {
            let (id, _) = item.split_once('=').expect("an item");
            assert!(id.ends_with(short), "{item} {short}");
        }
        assert_eq!(fields[6..8], ["markdown", "0"]);
        assert_eq!(fields[9..16], ["-", "-", "session", "0", "-", "-", "-"]);
        assert_eq!(fields[16..], ["note", "0", "-", "-", "-"]);
        assert_eq!(masked(own, &session), clean_line, "a clean run's line");
        assert!(!text.contains("d1\t5\tli"), "no byte of the tail");
    }
}

#[test]
fn a_run_that_can_write_neither_file_says_both() {
    let sandbox = Sandbox::new("digest_neither");
    sandbox.files();
    let target = sandbox.dir.join("elsewhere");
    fs::write(&target, "kept\n").expect("a target");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("a mode");
    symlink(&target, sandbox.mark_file()).expect("a link");
    symlink(&target, sandbox.log_file()).expect("a link");
    let run = sandbox.first(PROMPT);
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    let refused = |path: &Path| {
        format!(
            "{} is not a private file of this user in a folder of this user",
            path.display()
        )
    };
    assert_eq!(
        run.stderr,
        [
            format!(
                "bilbo: the session was not marked, so nothing was listed: {}",
                refused(&sandbox.mark_file())
            ),
            format!(
                "bilbo: the digest was not logged: {}",
                refused(&sandbox.log_file())
            ),
        ]
    );
    assert_eq!(fs::read_to_string(&target).expect("the target"), "kept\n");
}

#[test]
fn the_log_says_what_each_list_did() {
    let sandbox = Sandbox::new("digest_each_list");
    sandbox.files();
    sandbox.projects();
    sandbox.kept_text(KEPT);
    let prompt = "seed packets shed tomatoes north";
    let probing = fresh();
    let probe = sandbox.digest(&probing, prompt);
    let found = blocks(&probe.stdout);
    let session = &found
        .iter()
        .find(|(word, _)| word == "sessions")
        .expect("sessions")
        .1[0];
    let reference = session.split('\t').next().expect("a ref");
    let (_, range) = reference.split_once("#r").expect("a locator");
    assert_eq!(range, "2-7", "the fixture's own winning range");
    let (first, last) = (2, 7);
    let whole_parent = sandbox.entry_of(MAIN);
    let parent = Id::parse(&whole_parent).expect("an id");
    notes::noted(
        &sandbox.settings().layout(),
        parent,
        &[
            note_line::line(
                parent,
                "The beds were counted again",
                Some(1_794_039_249),
                &[(1, 1)],
            ),
            note_line::line(
                parent,
                "Tomatoes went into the north bed",
                Some(1_794_039_300),
                &[(first, last)],
            ),
        ],
    );
    let reindexed = sandbox.keep(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{:?}", reindexed.stderr);
    let floors = [
        ("BILBO_DIGEST_MARKDOWN_FLOOR", "0.1"),
        ("BILBO_DIGEST_SESSION_FLOOR", "0.2"),
        ("BILBO_DIGEST_NOTE_FLOOR", "0.3"),
    ];
    let floored = fresh();
    let run = sandbox.digest_with(&floored, &format!("{prompt} gazpacho"), &floors);
    assert_eq!(run.stderr, Vec::<String>::new());
    let log = sandbox.log();
    let line = log.last().expect("a line");
    assert_eq!(line.len(), 21, "{line:?}");
    assert_eq!(
        [line[6].as_str(), line[11].as_str(), line[16].as_str()],
        ["markdown", "session", "note"]
    );
    assert_eq!(
        [line[7].as_str(), line[12].as_str(), line[17].as_str()],
        ["0.1", "0.2", "0.3"]
    );
    let (item, score) = line[13].split_once('=').expect("an item");
    assert_eq!(item, format!("{whole_parent}#r2-7"), "{}", line[13]);
    assert!(score.ends_with('+') && score.trim_end_matches('+').parse::<f32>().is_ok());
    assert!(!line[13].contains(','), "one row");
    assert_eq!(line[15], "belongs=1");
    assert_eq!(line[20], "shown=1");
    let switched = fresh();
    let off = sandbox.digest_with(
        &switched,
        &format!("{prompt} gazpacho"),
        &[("BILBO_DIGEST_NOTE_MOST", "0")],
    );
    assert_eq!(off.stderr, Vec::<String>::new());
    let log = sandbox.log();
    let sessions: Vec<&str> = log.iter().map(|line| line[3].as_str()).collect();
    assert_eq!(sessions, [probing, floored, switched]);
    let line = log.last().expect("a line");
    assert_eq!(line.len(), 16);
    assert!(line[6..].chunks(5).all(|five| five[0] != "note"));
    let text = fs::read_to_string(sandbox.log_file()).expect("the log");
    sandbox
        .log()
        .iter()
        .for_each(|line| only_bilbos_words(line));
    for word in [
        "Tomatoes", "tomatoes", "gazpacho", "counted", "Plant", "commit", "packets", "seed",
        "shed", "Monday", "thinning", "carrots", "north", "hoe", "garden", "roof", "/home/me",
    ] {
        assert!(!text.contains(word), "{word}");
    }
}

impl Sandbox {
    fn of_kind(&self, kind: &str) -> Vec<(Id, String)> {
        use bilbo_store::entries::{Cursor, Entries};
        let store = self.held();
        let (changes, _) = store.changes(&Cursor::start()).expect("changes");
        let mut found: Vec<(Id, String)> = changes
            .iter()
            .map(bilbo_store::entries::Change::id)
            .filter_map(|id| {
                let meta = store.meta(id).expect("meta")?;
                (meta.kind == kind && meta.labels.id.is_none()).then(|| {
                    (
                        id,
                        meta.labels
                            .capture
                            .map(|label| label.as_str().to_owned())
                            .unwrap_or_default(),
                    )
                })
            })
            .collect();
        found.sort();
        found.dedup();
        found
    }

    fn sections(&self) -> Id {
        self.of_kind("markdown")
            .into_iter()
            .find(|(_, label)| label.ends_with("sections.md"))
            .expect("the garden log")
            .0
    }

    fn kept_note(&self) -> Id {
        self.of_kind("note").first().expect("a kept note").0
    }
}

type Planted = ([Id; 3], [Id; 3]);

fn planted(sandbox: &Sandbox) -> Planted {
    let layout = sandbox.settings().layout();
    let file = sandbox.sections();
    let session = Id::parse(&sandbox.entry_of(CLAUDE)).expect("an id");
    let note = sandbox.kept_note();
    let kept = [
        summaries::kept(&layout, file, "The beds since March."),
        summaries::kept(&layout, session, "Thinning the carrots."),
        summaries::kept(&layout, note, "Where the seeds are."),
    ];
    ([file, session, note], kept)
}

fn listed_of(line: &[String]) -> Vec<(String, Vec<String>, Vec<String>)> {
    let items = |field: &String| -> Vec<String> {
        if field == "-" {
            Vec::new()
        } else {
            field.split(',').map(str::to_owned).collect()
        }
    };
    line[6..]
        .chunks(5)
        .map(|five| (five[0].clone(), items(&five[2]), items(&five[3])))
        .collect()
}

fn summary_fields(stdout: &str) -> [Option<String>; 3] {
    let found = blocks(stdout);
    let last = |rows: Option<&String>| -> Option<String> {
        rows.and_then(|row| row.rsplit('\t').next())
            .filter(|field| field.starts_with("summary: "))
            .map(str::to_owned)
    };
    let list = |word: &str| {
        found
            .iter()
            .find(|(held, _)| held == word)
            .map(|(_, rows)| rows)
    };
    [
        last(list("files").and_then(|rows| rows.iter().find(|row| row.contains("sections.md")))),
        last(list("sessions").and_then(|rows| rows.first())),
        last(list("notes").and_then(|rows| rows.first())),
    ]
}

#[test]
fn a_file_a_session_and_a_kept_note_each_carry_their_summary() {
    let sandbox = full("digest_summaries");
    let ([file, session, note], _) = planted(&sandbox);
    let reindexed = sandbox.keep(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{:?}", reindexed.stderr);
    let run = sandbox.digest(&fresh(), PROMPT);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let found = blocks(&run.stdout);
    let words: Vec<&str> = found.iter().map(|(word, _)| word.as_str()).collect();
    assert_eq!(words, ["files", "sessions", "notes"]);
    let garden: Vec<&str> = found[0]
        .1
        .iter()
        .map(|row| row.split('\t').collect::<Vec<_>>())
        .find(|fields| fields[1].ends_with("sections.md"))
        .expect("the garden log's row")
        .into_iter()
        .collect();
    assert_eq!(garden.len(), 4, "{garden:?}");
    assert_eq!(garden[3], "summary: The beds since March.");
    let session_row: Vec<&str> = found[1].1[0].split('\t').collect();
    assert_eq!(
        session_row.last().copied(),
        Some("summary: Thinning the carrots.")
    );
    let note_row: Vec<&str> = found[2].1[0].split('\t').collect();
    assert_eq!(note_row.len(), 5, "{note_row:?}");
    assert_eq!(note_row[4], "summary: Where the seeds are.");
    let log = sandbox.log();
    let lists = listed_of(log.last().expect("a line"));
    let kinds: Vec<&str> = lists.iter().map(|(kind, _, _)| kind.as_str()).collect();
    assert_eq!(kinds, ["markdown", "session", "note"]);
    let (files, sessions, notes) = (&lists[0].1, &lists[1].1, &lists[2].1);
    let marked = |items: &Vec<String>, id: Id| -> usize {
        items
            .iter()
            .filter(|item| {
                item.starts_with(&id.to_string())
                    && item.ends_with('*')
                    && item[..item.len() - 1]
                        .split_once('=')
                        .is_some_and(|(_, score)| {
                            score.trim_end_matches('+').parse::<f32>().is_ok()
                        })
            })
            .count()
    };
    assert_eq!(marked(files, file), 1, "{files:?}");
    assert_eq!(marked(sessions, session), 1, "{sessions:?}");
    assert_eq!(marked(notes, note), 1, "{notes:?}");
    let shed = sandbox
        .of_kind("markdown")
        .into_iter()
        .find(|(_, label)| label.ends_with("shed.md"))
        .expect("the shed")
        .0;
    let bare: Vec<&String> = files
        .iter()
        .filter(|item| item.starts_with(&shed.to_string()))
        .collect();
    assert_eq!(bare.len(), 1, "{files:?}");
    let score = bare[0].split_once('=').expect("an item").1;
    assert!(score.parse::<f32>().is_ok(), "{score}");
    let held: Vec<&String> = lists.iter().flat_map(|(_, _, held)| held).collect();
    assert!(held.iter().all(|item| !item.contains('*')), "{held:?}");
    let every: usize = [files, sessions, notes]
        .iter()
        .map(|items| items.iter().filter(|item| item.ends_with('*')).count())
        .sum();
    assert_eq!(every, 3);
    assert_eq!(files.len() + sessions.len() + notes.len(), 4, "{lists:?}");
}

#[test]
fn with_the_switch_false_the_rows_are_those_of_a_store_with_no_summary() {
    let sandbox = full("digest_summaries_off");
    let before = sandbox.digest(&fresh(), PROMPT);
    assert!(before.stdout.starts_with("files:"));
    let baseline = sandbox.log().last().expect("a baseline line").clone();
    planted(&sandbox);
    let reindexed = sandbox.keep(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{:?}", reindexed.stderr);
    let off = [("BILBO_DIGEST_SUMMARY_ENABLED", "false")];
    let lines = sandbox.log().len();
    let run = sandbox.digest_with(&fresh(), PROMPT, &off);
    assert_eq!(
        (run.code, run.stdout.clone(), run.stderr.clone()),
        (0, before.stdout.clone(), Vec::<String>::new())
    );
    let log = sandbox.log();
    assert_eq!(log.len(), lines + 1);
    let new = log.last().expect("the new line");
    let mask = |line: &Vec<String>| -> Vec<String> {
        let mut masked = line.clone();
        for at in [1, 3, 4] {
            masked[at] = "-".to_owned();
        }
        masked
    };
    assert_eq!(mask(new), mask(&baseline));
    assert!(new.iter().all(|field| !field.contains('*')), "{new:?}");
    let on = sandbox.digest(&fresh(), PROMPT);
    assert_eq!(on.stdout.matches("\tsummary: ").count(), 3, "{}", on.stdout);
    assert_eq!(
        summary_fields(&on.stdout),
        [
            Some("summary: The beds since March.".to_owned()),
            Some("summary: Thinning the carrots.".to_owned()),
            Some("summary: Where the seeds are.".to_owned()),
        ],
        "{}",
        on.stdout
    );
}

#[test]
fn a_summary_is_printed_whole_however_long_it_is() {
    let sandbox = full("digest_summaries_whole");
    let layout = sandbox.settings().layout();
    let long: String = "b".repeat(1001);
    summaries::kept(&layout, sandbox.sections(), "one  two\\tthree\\nfour");
    summaries::kept(&layout, sandbox.kept_note(), &long);
    let reindexed = sandbox.keep(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{:?}", reindexed.stderr);
    let run = sandbox.digest(&fresh(), PROMPT);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let found = blocks(&run.stdout);
    let garden = found[0]
        .1
        .iter()
        .find(|row| row.contains("sections.md"))
        .expect("the garden log's row");
    assert!(
        garden.ends_with("\tsummary: one two three four"),
        "{garden}"
    );
    let notes = &found
        .iter()
        .find(|(word, _)| word == "notes")
        .expect("notes")
        .1;
    assert!(
        notes[0].ends_with(&format!("\tsummary: {long}")),
        "{notes:?}"
    );
}

#[test]
fn a_summarys_entry_is_never_a_row() {
    let sandbox = full("digest_summaries_never");
    let layout = sandbox.settings().layout();
    let summary = summaries::kept(&layout, sandbox.sections(), "Zucchini trellis.");
    let reindexed = sandbox.keep(&["reindex"]);
    assert_eq!(reindexed.code, 0, "{:?}", reindexed.stderr);
    let none = sandbox.digest(&fresh(), "zucchini trellis");
    assert_eq!(
        (none.code, none.stdout, none.stderr),
        (0, String::new(), Vec::<String>::new())
    );
    let usual = sandbox.digest(&fresh(), PROMPT);
    let files = file_rows(&usual.stdout);
    assert!(
        files.iter().any(|row| row[1].ends_with("sections.md")),
        "{}",
        usual.stdout
    );
    let (full_id, short) = (summary.to_string(), summary.short().to_string());
    assert!(!usual.stdout.contains(&full_id), "{}", usual.stdout);
    for row in usual.stdout.lines() {
        let reference = row.split('\t').next().unwrap_or_default();
        let id = reference.split('#').next().unwrap_or_default();
        assert_ne!(id, short, "{row}");
    }
}

fn agy_input(conversation: &str, transcript: &str, number: Option<&str>) -> Vec<u8> {
    let number = number.map_or_else(String::new, |number| format!(",\"invocationNum\":{number}"));
    format!(
        "{{\"conversationId\":\"{}\",\"transcriptPath\":\"{}\"{number}}}",
        escaped(conversation),
        escaped(transcript)
    )
    .into_bytes()
}

fn injected(plain: &str) -> String {
    let text = plain.strip_suffix('\n').expect("a line feed");
    format!(
        "{{\"injectSteps\":[{{\"systemMessage\":{{\"systemMessage\":\"{}\"}}}}]}}\n",
        escaped(text)
    )
}

fn user_step(text: &str) -> String {
    format!(
        "{{\"type\":\"USER_INPUT\",\"content\":\"{}\"}}\n",
        escaped(text)
    )
}

impl Sandbox {
    fn digest_agy(&self, input: &[u8]) -> sandbox::Run {
        sandbox::run(self.vars_with(&[]), &["digest", "--agy"], input, None)
    }

    fn transcript(&self, steps: &str) -> String {
        self.write("transcript.jsonl", steps.as_bytes());
        self.path("transcript.jsonl")
    }
}

#[test]
fn agy_gets_the_lists_as_one_json_line() {
    let sandbox = Sandbox::new("digest_agy_line");
    sandbox.files();
    let transcript = sandbox.transcript(&user_step(PROMPT));
    let conversation = fresh();
    let run = sandbox.digest_agy(&agy_input(&conversation, &transcript, Some("0")));
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    assert_eq!(run.stdout.matches('\n').count(), 1, "{}", run.stdout);
    assert!(run.stdout.ends_with('\n'));
    let plain = sandbox.first(PROMPT);
    assert!(plain.stdout.starts_with("files: 2 may bear"));
    assert_eq!(run.stdout, injected(&plain.stdout));
    let later = sandbox.digest_agy(&agy_input(&conversation, &transcript, None));
    assert_eq!(
        (later.code, later.stdout, later.stderr),
        (0, String::new(), Vec::<String>::new())
    );
}

#[test]
fn a_call_that_starts_no_turn_opens_no_state_folder_and_logs_nothing() {
    let sandbox = Sandbox::new("digest_agy_quiet");
    let run = sandbox.digest_agy(&agy_input(&fresh(), "/nowhere/t.jsonl", Some("3")));
    assert_eq!(
        (run.code, run.stdout, run.stderr, run.notices),
        (0, String::new(), Vec::<String>::new(), Vec::<String>::new())
    );
    assert!(!sandbox.dir.join("state").exists());
    assert!(!sandbox.log_file().exists());
    assert!(!sandbox.dir.join("store").exists());
}

#[test]
fn only_the_end_of_a_regular_transcript_is_read() {
    use bilbo_cli::digest::hook::{Heard, Refused, read_agy};
    use bilbo_cli::digest::transcript::{Unread, tail};

    let sandbox = Sandbox::new("digest_transcript");
    let limit = 1024u64;
    sandbox.write("short.jsonl", user_step("short one").as_bytes());
    let path = sandbox.path("short.jsonl");
    assert_eq!(
        tail(&path, limit).expect("a short file"),
        user_step("short one").into_bytes()
    );
    let filler = format!(
        "{{\"type\":\"SYSTEM_MESSAGE\",\"content\":\"{}\"}}\n",
        "x".repeat(200)
    );
    let long = format!("{}{}", user_step("far before"), filler.repeat(15));
    assert!(long.len() as u64 > 3 * limit);
    sandbox.write("long.jsonl", long.as_bytes());
    let path = sandbox.path("long.jsonl");
    let bytes = tail(&path, limit).expect("the end");
    assert_eq!(bytes, long.as_bytes()[long.len() - 1024..]);
    let input = agy_input("c", &path, None);
    let heard = read_agy(&mut &input[..], 4096, &|path| tail(path, limit));
    assert_eq!(heard, Err(Refused::NoUserInput("c".to_owned())));
    let nearer = format!("{}{}", filler.repeat(15), user_step("near"));
    sandbox.write("near.jsonl", nearer.as_bytes());
    let input = agy_input("c", &sandbox.path("near.jsonl"), None);
    let heard = read_agy(&mut &input[..], 4096, &|path| tail(path, limit));
    assert_eq!(heard.map(|heard| matches!(heard, Heard::Turn(_))), Ok(true));
    assert_eq!(tail(&sandbox.path("notes"), limit), Err(Unread::NotRegular));
    let missing = sandbox.path("missing.jsonl");
    assert!(
        matches!(tail(&missing, limit), Err(Unread::Failed(error)) if error.contains("No such file"))
    );
}

#[test]
fn an_agy_conversation_is_marked_and_matched_as_a_session_is() {
    let sandbox = full("digest_agy_own");
    let transcript = sandbox.transcript(&user_step(PROMPT));
    let run = sandbox.digest_agy(&agy_input(CLAUDE, &transcript, None));
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    let marks = String::from_utf8(sandbox.marks()).expect("text");
    assert!(marks.contains(CLAUDE), "{marks}");
    assert!(run.stdout.contains("files: 2 may bear"), "{}", run.stdout);
    assert!(run.stdout.contains("\\nnotes: "), "{}", run.stdout);
    assert!(!run.stdout.contains("sessions: "), "{}", run.stdout);
    let again = sandbox.digest_agy(&agy_input(CLAUDE, &transcript, None));
    assert_eq!(again.stdout, "");
}
