mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use common::{
    Fake, IDS, Locked, Run, TempDir, bench_store, bilbo, bilbo_input, config, guide, in_scope,
    library, snapshot, store, write,
};

const CREATED: &str = "2026-10-02T14:23-03:00";
const PROMPT: &str = "why does the embedder livelock on long chunks";

fn note(title: &str, body: &str) -> String {
    format!(
        "---\nid: {}\ncreated: {CREATED}\n---\n\n# {title}\n\n{body}",
        IDS[0]
    )
}

/// A note that holds three of `PROMPT`'s long words.
fn hit(rig: &Rig, name: &str) {
    write(
        &rig.root,
        name,
        &note("Slots", "The embedder livelocks, a livelock on chunks.\n"),
    );
}

fn payload(session: &str, prompt: &str) -> String {
    serde_json::json!({"session_id": session, "prompt": prompt, "hook_event_name": "UserPromptSubmit"})
        .to_string()
}

struct Rig {
    dir: TempDir,
    root: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(name: &str, lines: &[&str]) -> Rig {
        let dir = TempDir::new(name);
        let root = store(&dir);
        let config = config(&dir, lines);
        Rig { dir, root, config }
    }

    fn with_embedder(name: &str, fake: &Fake) -> Rig {
        let url = format!("embedder.url = {}", fake.url);
        Rig::new(
            name,
            &[
                url.as_str(),
                "embedder.model = test-model",
                "embedder.query_prefix = \"search: \"",
                "digest.log = on",
            ],
        )
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn sessions(&self) -> PathBuf {
        self.path("cache/bilbo/sessions")
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        let p = |s: &str| self.path(s).to_str().unwrap().to_string();
        vec![
            ("BILBO_HOME", self.root.to_str().unwrap().to_string()),
            ("BILBO_CONFIG", self.config.to_str().unwrap().to_string()),
            ("XDG_CACHE_HOME", p("cache")),
            ("XDG_STATE_HOME", p("state")),
            ("HOME", p("home")),
        ]
    }

    fn feed(&self, input: &str) -> Run {
        let env = self.env();
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let run = bilbo_input(self.dir.path(), &env, &["digest"], input);
        assert_eq!(run.code, 0, "{}", run.stderr);
        run
    }

    fn digest(&self, session: &str, prompt: &str) -> Run {
        self.feed(&payload(session, prompt))
    }

    fn index(&self) {
        let env = self.env();
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let run = bilbo(self.dir.path(), &env, &["index"]);
        assert_eq!(run.code, 0, "{}", run.stderr);
    }

    fn log(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.path("state/bilbo/digest.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// Asserts the run printed nothing but one `bilbo: ` line containing `part`.
fn refused(run: &Run, part: &str) {
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    assert!(
        run.stderr.starts_with("bilbo: ") && run.stderr.contains(part),
        "{}",
        run.stderr
    );
}

fn silent(run: &Run) {
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

fn backdate(path: &Path, days: u64) {
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60))
        .unwrap();
}

/// A note of its own id that holds three of `PROMPT`'s long words.
fn sync_note(rig: &Rig, name: &str, id: &str) {
    write(
        &rig.root,
        name,
        &format!(
            "---\nid: {id}\ncreated: {CREATED}\n---\n\n# Slots\n\nThe embedder livelocks, a livelock on chunks.\n"
        ),
    );
}

fn id(n: usize) -> String {
    format!("01M3YJ7R6HK6NQ30DCDB1P{n:04}")
}

/// An `open.json` entry with one open conflict.
fn conflict(file: &str) -> serde_json::Value {
    serde_json::json!({
        "file": file,
        "conflict": [{"version": "v1", "passage": "Slots", "sides": ["v2", "v3"]}],
    })
}

fn open_json(rig: &Rig, entries: serde_json::Value) {
    let dir = rig.root.join(".bilbo/sync");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("open.json"),
        serde_json::json!({"notes": entries}).to_string(),
    )
    .unwrap();
}

fn waits(rig: &Rig, names: &[&str]) -> String {
    let paths: Vec<String> = names
        .iter()
        .map(|n| format!("{}/notes/{n}", rig.root.display()))
        .collect();
    format!(
        "Sync conflicts wait in: {} (run bilbo check)",
        paths.join(", ")
    )
}

#[test]
fn input_claude_payload() {
    let rig = Rig::new("digest-claude", &[]);
    hit(&rig, "gotcha-slots.md");
    let input = serde_json::json!({
        "session_id": "abc-123",
        "prompt": PROMPT,
        "cwd": "/tmp",
        "hook_event_name": "UserPromptSubmit",
    })
    .to_string();
    let run = rig.feed(&input);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
}

#[test]
fn input_codex_payload() {
    let rig = Rig::new("digest-codex", &[]);
    hit(&rig, "gotcha-slots.md");
    let input = serde_json::json!({
        "session_id": "01a102b8-7eb6-7a50-b5e7-e58145afdffd",
        "turn_id": "t1",
        "prompt": PROMPT,
        "model": "m",
        "hook_event_name": "UserPromptSubmit",
    })
    .to_string();
    let run = rig.feed(&input);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
}

#[test]
fn input_garbage() {
    let rig = Rig::new("digest-garbage", &[]);
    hit(&rig, "gotcha-slots.md");
    refused(&rig.feed("not json"), "the hook input is not a JSON object");
}

#[test]
fn input_array_is_not_an_object() {
    let rig = Rig::new("digest-array", &[]);
    hit(&rig, "gotcha-slots.md");
    refused(
        &rig.feed(r#"[{"session_id": "abc", "prompt": "x"}]"#),
        "the hook input is not a JSON object",
    );
}

#[test]
fn input_missing_prompt() {
    let rig = Rig::new("digest-no-prompt", &["digest.log = on"]);
    hit(&rig, "gotcha-slots.md");
    refused(
        &rig.feed(r#"{"session_id": "abc"}"#),
        "the hook input has no prompt",
    );
    refused(
        &rig.feed(r#"{"session_id": "abc", "prompt": ""}"#),
        "the hook input has no prompt",
    );
    refused(
        &rig.feed(&format!(r#"{{"prompt": "{PROMPT}"}}"#)),
        "the hook input has no session_id",
    );
}

#[test]
fn input_session_with_a_slash() {
    let rig = Rig::new("digest-slash", &["digest.log = on"]);
    hit(&rig, "gotcha-slots.md");
    let outside = |rig: &Rig| {
        let mut seen = snapshot(rig.dir.path());
        seen.retain(|path, _| {
            path != rig.dir.path()
                && !path.starts_with(rig.path("cache"))
                && !path.starts_with(rig.path("state"))
        });
        seen
    };
    let before = outside(&rig);
    refused(
        &rig.digest("../../etc", PROMPT),
        "the hook input's session_id is not 1 to 128",
    );
    assert_eq!(outside(&rig), before);
    for path in snapshot(rig.dir.path()).keys() {
        assert_ne!(path.file_name().unwrap(), "etc", "{}", path.display());
    }
    assert!(!rig.sessions().exists());
}

#[test]
fn input_dot_dot_session() {
    let rig = Rig::new("digest-dotdot", &[]);
    hit(&rig, "gotcha-slots.md");
    for id in ["..", "."] {
        refused(&rig.digest(id, PROMPT), "session_id");
    }
    assert!(!rig.path("cache").exists());
}

#[test]
fn query_command_prompt() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-command", &fake);
    hit(&rig, "gotcha-slots.md");
    rig.index();
    let run = rig.digest("abc", "/deploy:run release-notes");
    assert_eq!(run.code, 0);
    assert_eq!(
        fake.inputs().last().unwrap(),
        "search: deploy:run release-notes"
    );
}

#[test]
fn query_path_is_not_a_command() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-path", &fake);
    hit(&rig, "gotcha-slots.md");
    rig.index();
    let before = fake.requests().len();
    silent(&rig.digest("abc", "/Users/a/notes/plan.md what is this?"));
    assert_eq!(fake.requests().len(), before);
    let last = rig.log().pop().unwrap();
    assert_eq!(last["ranking"], "none");
}

#[test]
fn gate_close_in_meaning() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-close", &fake);
    write(
        &rig.root,
        "decision-note-store.md",
        &note("Note store", "One flat folder of files.\n"),
    );
    fake.vector("flat folder", &[0.62, 0.7846, 0.0]);
    fake.vector("where", &[1.0, 0.0, 0.0]);
    rig.index();
    let run = rig.digest("abc", "where do the notes live");
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(
        run.stdout
            .starts_with("<!-- bilbo digest: 1 of 1 notes -->\n"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("decision-note-store.md"));
    assert_eq!(rig.log().pop().unwrap()["ranking"], "meaning");
}

#[test]
fn gate_shared_words_do_not_override_a_weak_similarity() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-weak", &fake);
    write(
        &rig.root,
        "plan-specs.md",
        &note("Specs", "Work on the specs where they live.\n"),
    );
    fake.vector("Work on the specs", &[0.41, 0.0, 0.9121]);
    fake.vector("where", &[1.0, 0.0, 0.0]);
    rig.index();
    silent(&rig.digest("abc", "where do the work specs live"));
}

#[test]
fn gate_keyword_without_an_embedder() {
    let rig = Rig::new("digest-keyword", &[]);
    hit(&rig, "gotcha-slots.md");
    let run = rig.digest("abc", PROMPT);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
}

#[test]
fn gate_two_words_are_not_enough() {
    let rig = Rig::new("digest-two-words", &[]);
    write(
        &rig.root,
        "gotcha-slots.md",
        &note("Slots", "The embedder livelocks on a livelock.\n"),
    );
    silent(&rig.digest("abc", PROMPT));
}

#[test]
fn gate_nothing_indexed_asks_no_embedder() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-unindexed", &fake);
    hit(&rig, "gotcha-slots.md");
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
    assert_eq!(
        run.stderr,
        "bilbo: no passage is indexed; run bilbo index\n"
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn limit_once_per_session() {
    let rig = Rig::new("digest-once", &[]);
    hit(&rig, "gotcha-first.md");
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.contains("gotcha-first.md"), "{}", run.stdout);
    silent(&rig.digest("abc", PROMPT));
    hit(&rig, "gotcha-second.md");
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.contains("gotcha-second.md"), "{}", run.stdout);
    assert!(!run.stdout.contains("gotcha-first.md"), "{}", run.stdout);
    assert!(
        run.stdout
            .starts_with("<!-- bilbo digest: 1 of 1 notes -->\n")
    );
}

#[test]
fn limit_later_prompt_shows_three() {
    let rig = Rig::new("digest-later", &[]);
    hit(&rig, "gotcha-first.md");
    rig.digest("abc", PROMPT);
    for i in 0..5 {
        hit(&rig, &format!("gotcha-more-{i}.md"));
    }
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout
            .starts_with("<!-- bilbo digest: 3 of 5 notes -->\n"),
        "{}",
        run.stdout
    );
    assert_eq!(
        run.stdout.lines().filter(|l| l.starts_with("- ")).count(),
        3
    );
    assert!(
        run.stdout
            .ends_with("(2 more passed; run bilbo recall for them)\n")
    );
}

#[test]
fn limit_new_session_starts_fresh() {
    let rig = Rig::new("digest-fresh", &[]);
    hit(&rig, "gotcha-slots.md");
    assert!(rig.digest("abc", PROMPT).stdout.contains("gotcha-slots.md"));
    assert!(rig.digest("def", PROMPT).stdout.contains("gotcha-slots.md"));
}

#[test]
fn limit_block_too_large_for_six() {
    let rig = Rig::new("digest-large", &[]);
    let title = "x".repeat(2000);
    for i in 0..6 {
        write(
            &rig.root,
            &format!("gotcha-large-{i}.md"),
            &note(&title, "The embedder livelocks, a livelock on chunks.\n"),
        );
    }
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout
            .starts_with("<!-- bilbo digest: 4 of 6 notes -->\n"),
        "{}",
        &run.stdout[..80.min(run.stdout.len())]
    );
    assert!(run.stdout.len() <= 9000, "{}", run.stdout.len());
    assert_eq!(
        run.stdout.lines().filter(|l| l.starts_with("- ")).count(),
        4
    );
}

#[test]
fn limit_nothing_passes() {
    let rig = Rig::new("digest-nothing", &[]);
    write(&rig.root, "gotcha-slots.md", &note("Slots", "Unrelated.\n"));
    silent(&rig.digest("abc", PROMPT));
}

#[test]
fn block_first_digest() {
    let rig = Rig::new("digest-block", &[]);
    for i in 0..8 {
        hit(&rig, &format!("gotcha-slot-{i}.md"));
    }
    let run = rig.digest("abc", PROMPT);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 9, "{}", run.stdout);
    assert_eq!(lines[0], "<!-- bilbo digest: 6 of 8 notes -->");
    assert_eq!(
        lines[1],
        "Notes that may bear on this prompt (open the file to read more):"
    );
    assert!(lines[2..8].iter().all(|l| l.starts_with("- ")));
    assert_eq!(
        lines[2],
        format!(
            "- {}/notes/gotcha-slot-0.md:6 (gotcha, {CREATED}) Slots: The embedder livelocks, a livelock on chunks.",
            rig.root.display()
        )
    );
    assert_eq!(lines[8], "(2 more passed; run bilbo recall for them)");
}

#[test]
fn block_no_overflow_line() {
    let rig = Rig::new("digest-no-overflow", &[]);
    hit(&rig, "gotcha-slot-0.md");
    hit(&rig, "gotcha-slot-1.md");
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout
            .starts_with("<!-- bilbo digest: 2 of 2 notes -->\n"),
        "{}",
        run.stdout
    );
    assert_eq!(run.stdout.lines().count(), 4);
    assert!(!run.stdout.contains("more passed"));
}

#[test]
fn memory_file_names_the_notes() {
    let rig = Rig::new("digest-memory", &[]);
    hit(&rig, "gotcha-slot-0.md");
    hit(&rig, "gotcha-slot-1.md");
    rig.digest("abc", PROMPT);
    let text = std::fs::read_to_string(rig.sessions().join("abc")).unwrap();
    let mut paths: Vec<&str> = text.lines().collect();
    paths.sort();
    let root = rig.root.display();
    assert_eq!(
        paths,
        [
            format!("{root}/notes/gotcha-slot-0.md"),
            format!("{root}/notes/gotcha-slot-1.md")
        ]
    );
    let names: Vec<String> = std::fs::read_dir(rig.sessions())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["abc"]);
}

#[test]
fn memory_nothing_shown_nothing_remembered() {
    let rig = Rig::new("digest-forget", &[]);
    write(&rig.root, "gotcha-slots.md", &note("Slots", "Unrelated.\n"));
    silent(&rig.digest("abc", PROMPT));
    assert!(!rig.sessions().join("abc").exists());
    for i in 0..8 {
        hit(&rig, &format!("gotcha-slot-{i}.md"));
    }
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout
            .starts_with("<!-- bilbo digest: 6 of 8 notes -->\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn memory_old_sessions_are_swept() {
    let rig = Rig::new("digest-sweep", &[]);
    std::fs::create_dir_all(rig.sessions()).unwrap();
    for name in ["old", "recent"] {
        std::fs::write(rig.sessions().join(name), "").unwrap();
    }
    backdate(&rig.sessions().join("old"), 31);
    backdate(&rig.sessions().join("recent"), 29);
    silent(&rig.digest("abc", PROMPT));
    assert!(!rig.sessions().join("old").exists());
    assert!(rig.sessions().join("recent").exists());
}

#[test]
fn fail_no_store() {
    let dir = TempDir::new("digest-no-store");
    let root = dir.path().join("missing");
    let config = config(&dir, &[]);
    let rig = Rig { dir, root, config };
    let run = rig.digest("abc", PROMPT);
    refused(&run, &format!("no store at {}", rig.root.display()));
}

#[test]
fn fail_broken_config() {
    let rig = Rig::new("digest-broken", &["digest.bogus = 1", "digest.log = on"]);
    hit(&rig, "gotcha-slots.md");
    refused(&rig.digest("abc", PROMPT), "digest.bogus");
    assert!(rig.log().is_empty());
}

#[test]
fn fail_missing_explicit_config() {
    let mut rig = Rig::new("digest-missing-config", &[]);
    hit(&rig, "gotcha-slots.md");
    rig.config = rig.path("nope");
    let run = rig.digest("abc", PROMPT);
    refused(&run, &rig.config.display().to_string());
}

#[test]
fn fail_unwritable_cache() {
    let rig = Rig::new("digest-unwritable", &[]);
    hit(&rig, "gotcha-slots.md");
    std::fs::create_dir_all(rig.path("cache/bilbo")).unwrap();
    std::fs::write(rig.sessions(), "").unwrap();
    let run = rig.digest("abc", PROMPT);
    refused(&run, &format!("cannot create {}", rig.sessions().display()));
}

#[test]
fn fail_embedder_refuses() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-refuses", &fake);
    hit(&rig, "gotcha-slots.md");
    rig.index();
    fake.status(500);
    let run = rig.digest("abc", PROMPT);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    assert!(
        run.stderr.contains(&fake.url) && run.stderr.contains("500"),
        "{}",
        run.stderr
    );
}

#[test]
fn switch_off_prints_and_writes_nothing() {
    let rig = Rig::new("digest-off", &["digest.enable = off", "digest.log = on"]);
    hit(&rig, "gotcha-slots.md");
    silent(&rig.digest("abc", PROMPT));
    assert!(!rig.sessions().exists());
    assert!(!rig.path("state/bilbo").exists());

    std::fs::create_dir_all(rig.sessions()).unwrap();
    std::fs::write(rig.sessions().join("old"), "").unwrap();
    backdate(&rig.sessions().join("old"), 31);
    silent(&rig.digest("abc", PROMPT));
    let names: Vec<String> = std::fs::read_dir(rig.sessions())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["old"]);
    assert!(!rig.path("state/bilbo").exists());
}

#[test]
fn switch_on_by_default() {
    let rig = Rig::new("digest-on", &[]);
    hit(&rig, "gotcha-slots.md");
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
}

#[test]
fn budget_stalled_embedder() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-stalled", &fake);
    hit(&rig, "gotcha-slots.md");
    rig.index();
    fake.stall();
    let asked = fake.requests().len();
    let start = Instant::now();
    let run = rig.digest("abc", PROMPT);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(1500),
        "{elapsed:?}: {}",
        run.stderr
    );
    assert_eq!(
        fake.requests().len(),
        asked + 1,
        "the embedder was not asked"
    );
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    let line = rig.log().pop().unwrap();
    assert_eq!(line["ranking"], "keywords");
    assert_eq!(line["passed"], 1);
    let error = line["error"].as_str().unwrap();
    assert!(error.contains("did not answer within"), "{error}");
    assert!(line["elapsed_ms"].as_u64().unwrap() < 1500);
}

#[test]
fn budget_long_prompt() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-long", &fake);
    hit(&rig, "gotcha-slots.md");
    rig.index();
    let prompt: String = PROMPT.chars().cycle().take(5000).collect();
    assert_eq!(prompt.len(), 5000);
    let asked = fake.requests().len();
    rig.digest("abc", &prompt);
    assert_eq!(
        fake.requests().len(),
        asked + 1,
        "the embedder was not asked"
    );
    assert_eq!(
        fake.inputs().last().unwrap(),
        &format!("search: {}", &prompt[..1000])
    );
}

#[test]
fn log_empty_digest_is_explained() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-log-empty", &fake);
    write(
        &rig.root,
        "gotcha-slots.md",
        &note("Slots", "The embedder livelocks on a livelock.\n"),
    );
    rig.index();
    fake.stall();
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    let log = rig.log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["ranking"], "keywords");
    assert_eq!(log[0]["passed"], 0);
    assert_eq!(log[0]["shown"], serde_json::json!([]));
    assert!(
        log[0]["error"]
            .as_str()
            .unwrap()
            .contains("did not answer within"),
        "{}",
        log[0]
    );
}

#[test]
fn log_digest_that_showed_notes() {
    let rig = Rig::new("digest-log-shown", &["digest.log = on"]);
    hit(&rig, "gotcha-slots.md");
    hit(&rig, "gotcha-more-slots.md");
    let run = rig.digest("abc", PROMPT);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    let log = rig.log();
    assert_eq!(log.len(), 1);
    let line = &log[0];
    assert_eq!(line["session"], "abc");
    assert_eq!(line["prompt"], PROMPT);
    assert_eq!(line["ranking"], "keywords");
    assert_eq!(line["passed"], 2);
    assert!(line["elapsed_ms"].is_u64());
    assert!(line["time"].as_str().unwrap().contains('T'));
    assert!(line.get("error").is_none(), "{line}");
    let mut shown: Vec<&str> = line["shown"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    shown.sort();
    assert_eq!(shown.len(), 2);
    assert!(shown[0].ends_with("gotcha-more-slots.md"), "{shown:?}");
    assert!(shown[1].ends_with("gotcha-slots.md"), "{shown:?}");
    let mode = std::fs::metadata(rig.path("state/bilbo/digest.jsonl"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn log_off_by_default() {
    let rig = Rig::new("digest-log-off", &[]);
    hit(&rig, "gotcha-slots.md");
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
    assert!(!rig.path("state/bilbo").exists());
}

#[test]
fn log_path_prompt_has_ranking_none() {
    let rig = Rig::new("digest-log-path", &["digest.log = on"]);
    hit(&rig, "gotcha-slots.md");
    let prompt = "/Users/a/notes/plan.md what is this?";
    silent(&rig.digest("abc", prompt));
    let log = rig.log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["ranking"], "none");
    assert_eq!(log[0]["passed"], 0);
    assert_eq!(log[0]["shown"], serde_json::json!([]));
    assert_eq!(log[0]["prompt"], prompt);
}

#[test]
fn store_and_cache_left_alone() {
    let fake = Fake::start(3);
    let rig = Rig::with_embedder("digest-untouched", &fake);
    write(
        &rig.root,
        "decision-note-store.md",
        &note("Note store", "One flat folder of files.\n"),
    );
    fake.vector("flat folder", &[0.62, 0.7846, 0.0]);
    fake.vector("where", &[1.0, 0.0, 0.0]);
    rig.index();
    let cache = rig.path("cache/bilbo");
    let vectors = |dir: &Path| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        files
    };
    let files = vectors(&cache);
    assert!(!files.is_empty(), "the index wrote no vector cache");
    let stat = |files: &[PathBuf]| -> Vec<(Vec<u8>, SystemTime)> {
        files
            .iter()
            .map(|f| {
                (
                    std::fs::read(f).unwrap(),
                    std::fs::metadata(f).unwrap().modified().unwrap(),
                )
            })
            .collect()
    };
    let before = (snapshot(&rig.root), stat(&files));
    let run = rig.digest("abc", "where do the notes live");
    assert!(
        run.stdout.contains("decision-note-store.md"),
        "{}",
        run.stdout
    );
    assert_eq!(rig.log().pop().unwrap()["ranking"], "meaning");
    assert_eq!(vectors(&cache), files);
    assert_eq!((snapshot(&rig.root), stat(&files)), before);
}

#[test]
#[ignore = "timing; run with --release -- --ignored"]
fn digest_over_a_6_mib_store_is_fast() {
    let dir = TempDir::new("digest-speed");
    let (root, mib) = bench_store(&dir);
    let fake = Fake::start(1024);
    let url = format!("embedder.url = {}", fake.url);
    let config = config(&dir, &[url.as_str(), "embedder.model = test-model"]);
    let rig = Rig { dir, root, config };
    rig.index();
    let mut entries = serde_json::Map::new();
    for n in 0..400 {
        let mut entry = conflict(&format!("gotcha-bench-{n}.md"));
        entry["dropped"] = serde_json::json!([{
            "conflict": "v1",
            "passage": "Section 0 > Part 1",
            "lines": vec!["a dropped line of about eighty characters, kept in the summary as is"; 120],
        }]);
        entries.insert(if n == 0 { IDS[0].to_string() } else { id(n) }, entry);
    }
    open_json(&rig, entries.into());
    fake.delay(Duration::from_millis(100));
    let prompt = "embedder timeout decisao";
    rig.digest("warm", prompt);
    for i in 0..3 {
        let start = Instant::now();
        let run = rig.digest(&format!("s{i}"), prompt);
        let elapsed = start.elapsed();
        eprintln!(
            "store {mib:.1} MiB: digest took {} ms ({} lines)",
            elapsed.as_millis(),
            run.stdout.lines().count()
        );
        assert!(
            run.stdout.starts_with("<!-- bilbo digest:"),
            "{}",
            run.stdout
        );
        assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(400), "{elapsed:?}");
    }
}

#[test]
fn a_source_holding_every_word_is_not_a_digest_hit() {
    let rig = Rig::new("digest-library", &[]);
    write(
        &rig.root,
        "plan-a.md",
        &note("Plan a", "Nothing relevant.\n"),
    );
    for n in 0..5 {
        library(
            &rig.root,
            "go",
            &format!("source-{n}"),
            "# Slots\n\nWhy does the embedder livelock on long chunks, a livelock on chunks.\n",
        );
    }
    guide(
        &rig.root,
        "go",
        "About go.",
        &[("source-0", "the embedder livelock on long chunks")],
    );
    let before = snapshot(&rig.root.join("library"));
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(snapshot(&rig.root.join("library")), before);

    let locked = Locked::new(&rig.root.join("library"), 0o755);
    let again = rig.digest("def", PROMPT);
    drop(locked);
    assert!(again.stdout.is_empty(), "{}", again.stdout);
    assert!(again.stderr.is_empty(), "{}", again.stderr);
}

const DEPLOY_PROMPT: &str = "why does the deploy pipeline stall on staging";

/// The config lines for a remote embedder at the fake's `0.0.0.0` address, `scopes` after the standard ones.
fn remote_lines(fake: &Fake, scopes: &[&str]) -> Vec<String> {
    let mut lines = vec![
        format!("embedder.url = http://0.0.0.0:{}", fake.port()),
        "embedder.model = test-model".to_string(),
        "embedder.query_prefix = \"search: \"".to_string(),
        "digest.log = on".to_string(),
    ];
    lines.extend(scopes.iter().map(|s| s.to_string()));
    lines
}

fn remote_rig(name: &str, fake: &Fake, scopes: &[&str]) -> Rig {
    let lines = remote_lines(fake, scopes);
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    Rig::new(name, &lines)
}

/// A store whose one `any` note is embedded, so the digest's meaning gate is in charge, and a query vector
/// that is far from it.
fn anchored(name: &str, fake: &Fake, scopes: &[&str]) -> Rig {
    fake.vector("why does", &[1.0, 0.0, 0.0]);
    fake.vector("Anchor", &[0.0, 1.0, 0.0]);
    let rig = remote_rig(name, fake, scopes);
    write(
        &rig.root,
        "plan-anchor.md",
        &in_scope(&note("Anchor", "Nothing to see here.\n"), "personal"),
    );
    rig
}

const WORK_LOCAL: &[&str] = &[
    "scope.work.embedder = local",
    "scope.personal.embedder = any",
];

#[test]
fn gate_withheld_note_passes_on_keywords() {
    let fake = Fake::start(3);
    let rig = anchored("digest-withheld-pass", &fake, WORK_LOCAL);
    write(
        &rig.root,
        "plan-deploy.md",
        &in_scope(
            &note("Deploy", "The deploy pipeline runs on staging.\n"),
            "work",
        ),
    );
    rig.index();
    let run = rig.digest("abc", DEPLOY_PROMPT);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(run.stdout.contains("plan-deploy.md"), "{}", run.stdout);
    assert!(!run.stdout.contains("plan-anchor.md"), "{}", run.stdout);
    assert_eq!(rig.log().pop().unwrap()["ranking"], "meaning");
    assert!(
        fake.inputs().iter().all(|i| !i.contains("runs on staging")),
        "{:?}",
        fake.inputs()
    );
}

#[test]
fn gate_withheld_note_needs_three_words() {
    let fake = Fake::start(3);
    let rig = anchored("digest-withheld-two", &fake, WORK_LOCAL);
    write(
        &rig.root,
        "plan-deploy.md",
        &in_scope(&note("Deploy", "A deploy waits on staging.\n"), "work"),
    );
    rig.index();
    silent(&rig.digest("abc", DEPLOY_PROMPT));
    assert_eq!(rig.log().pop().unwrap()["ranking"], "meaning");
}

#[test]
fn gate_unembedded_note_in_no_local_scope_still_needs_meaning() {
    let fake = Fake::start(3);
    let rig = anchored(
        "digest-unembedded",
        &fake,
        &["scope.personal.embedder = any"],
    );
    rig.index();
    write(
        &rig.root,
        "plan-deploy.md",
        &in_scope(
            &note("Deploy", "The deploy pipeline runs on staging.\n"),
            "personal",
        ),
    );
    silent(&rig.digest("abc", DEPLOY_PROMPT));
    assert_eq!(rig.log().pop().unwrap()["ranking"], "meaning");
}

#[test]
fn gate_ignores_the_cached_vector_of_a_withheld_note() {
    let fake = Fake::start(3);
    fake.vector("why does", &[1.0, 0.0, 0.0]);
    fake.vector("Anchor", &[0.0, 1.0, 0.0]);
    fake.vector("Deploy", &[1.0, 0.0, 0.0]);
    let rig = remote_rig(
        "digest-stale-vector",
        &fake,
        &["scope.personal.embedder = any"],
    );
    write(
        &rig.root,
        "plan-anchor.md",
        &in_scope(&note("Anchor", "Nothing to see here.\n"), "personal"),
    );
    write(
        &rig.root,
        "plan-deploy.md",
        &in_scope(&note("Deploy", "A deploy waits on staging.\n"), "work"),
    );
    rig.index();
    let run = rig.digest("abc", DEPLOY_PROMPT);
    assert!(run.stdout.contains("plan-deploy.md"), "{}", run.stdout);

    let lines = remote_lines(&fake, WORK_LOCAL);
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    config(&rig.dir, &lines);
    silent(&rig.digest("def", DEPLOY_PROMPT));
    assert_eq!(rig.log().pop().unwrap()["ranking"], "meaning");
}

#[test]
fn gate_an_all_withheld_store_ranks_by_keywords_and_asks_nothing() {
    let fake = Fake::start(3);
    let rig = remote_rig(
        "digest-all-withheld",
        &fake,
        &["scope.work.embedder = local"],
    );
    write(
        &rig.root,
        "plan-deploy.md",
        &in_scope(
            &note("Deploy", "The deploy pipeline runs on staging.\n"),
            "work",
        ),
    );
    rig.index();
    let run = rig.digest("abc", DEPLOY_PROMPT);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(run.stdout.contains("plan-deploy.md"), "{}", run.stdout);
    let last = rig.log().pop().unwrap();
    assert_eq!(last["ranking"], "keywords");
    assert!(last.get("error").is_none_or(|e| e.is_null()), "{last}");
    assert!(fake.requests().is_empty());
}

#[test]
fn sync_a_conflicted_note_is_labelled() {
    let rig = Rig::new("digest-sync-conflict", &[]);
    sync_note(&rig, "gotcha-nix.md", &id(1));
    open_json(
        &rig,
        serde_json::json!({ id(1): conflict("gotcha-nix.md") }),
    );
    let run = rig.digest("abc", PROMPT);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{}", run.stdout);
    assert_eq!(lines[0], "<!-- bilbo digest: 1 of 1 notes -->");
    assert_eq!(
        lines[2],
        format!(
            "- {}/notes/gotcha-nix.md:6 (gotcha, {CREATED}, conflict) Slots: The embedder livelocks, a livelock on chunks.",
            rig.root.display()
        )
    );
    assert_eq!(lines[3], waits(&rig, &["gotcha-nix.md"]));
}

#[test]
fn sync_undeclared_dropped_text_is_a_conflict_too() {
    let rig = Rig::new("digest-sync-dropped", &[]);
    sync_note(&rig, "gotcha-nix.md", &id(1));
    open_json(
        &rig,
        serde_json::json!({ id(1): {
            "file": "gotcha-nix.md",
            "dropped": [{"conflict": "v1", "passage": "Slots", "lines": ["a line"]}],
        }}),
    );
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout
            .contains(&format!("(gotcha, {CREATED}, conflict)")),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("Sync conflicts wait in:"),
        "{}",
        run.stdout
    );
}

#[test]
fn sync_an_auto_merged_note_is_labelled_and_raises_nothing() {
    let rig = Rig::new("digest-sync-merged", &[]);
    sync_note(&rig, "plan-release.md", &id(1));
    open_json(
        &rig,
        serde_json::json!({ id(1): {"file": "plan-release.md", "merged": true} }),
    );
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout
            .contains(&format!("(plan, {CREATED}, auto-merged)")),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("Sync conflicts"), "{}", run.stdout);
}

#[test]
fn sync_a_waiting_note_that_was_also_merged_reads_conflict() {
    let rig = Rig::new("digest-sync-both", &[]);
    sync_note(&rig, "gotcha-nix.md", &id(1));
    let mut entry = conflict("gotcha-nix.md");
    entry["merged"] = true.into();
    open_json(&rig, serde_json::json!({ id(1): entry }));
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.contains(", conflict)"), "{}", run.stdout);
    assert!(!run.stdout.contains("auto-merged"), "{}", run.stdout);
}

#[test]
fn sync_a_notice_alone_labels_nothing() {
    let rig = Rig::new("digest-sync-notice", &[]);
    sync_note(&rig, "gotcha-nix.md", &id(1));
    open_json(
        &rig,
        serde_json::json!({ id(1): {
            "file": "gotcha-nix.md",
            "notices": [{"at": "2026-10-04T10:00:00Z", "flag": "a flag"}],
            "left": [{"scope": "work", "at": "2026-10-01T10:00:00Z"}],
        }}),
    );
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout.contains(&format!("(gotcha, {CREATED}) ")),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("Sync conflicts"), "{}", run.stdout);
}

#[test]
fn sync_conflicts_alone_follow_the_empty_header() {
    let rig = Rig::new("digest-sync-alone", &[]);
    write(&rig.root, "gotcha-nix.md", &note("Nix", "Unrelated.\n"));
    open_json(
        &rig,
        serde_json::json!({ IDS[0]: conflict("gotcha-nix.md") }),
    );
    let run = rig.digest("abc", PROMPT);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "<!-- bilbo digest: 0 of 0 notes -->\n{}\n",
            waits(&rig, &["gotcha-nix.md"])
        )
    );
}

#[test]
fn sync_conflicts_are_raised_once_per_session() {
    let rig = Rig::new("digest-sync-once", &[]);
    sync_note(&rig, "gotcha-nix.md", &id(1));
    open_json(
        &rig,
        serde_json::json!({ id(1): conflict("gotcha-nix.md") }),
    );
    let first = rig.digest("abc", PROMPT);
    assert!(
        first.stdout.contains("Sync conflicts wait in"),
        "{}",
        first.stdout
    );
    sync_note(&rig, "gotcha-other.md", &id(2));
    let later = rig.digest("abc", PROMPT);
    assert!(later.stdout.contains("gotcha-other.md"), "{}", later.stdout);
    assert!(!later.stdout.contains("Sync conflicts"), "{}", later.stdout);
    assert!(!later.stdout.contains("gotcha-nix.md"), "{}", later.stdout);
    let other = rig.digest("def", PROMPT);
    assert!(
        other.stdout.contains("Sync conflicts wait in"),
        "{}",
        other.stdout
    );
}

#[test]
fn sync_raised_conflicts_are_remembered() {
    let rig = Rig::new("digest-sync-remembered", &[]);
    write(&rig.root, "gotcha-nix.md", &note("Nix", "Unrelated.\n"));
    open_json(
        &rig,
        serde_json::json!({ IDS[0]: conflict("gotcha-nix.md") }),
    );
    let first = rig.digest("abc", PROMPT);
    assert!(
        first.stdout.contains("Sync conflicts wait in"),
        "{}",
        first.stdout
    );
    assert!(rig.sessions().join("abc").is_file());
    silent(&rig.digest("abc", PROMPT));
    sync_note(&rig, "gotcha-slots.md", &id(2));
    let later = rig.digest("abc", PROMPT);
    assert!(later.stdout.contains("gotcha-slots.md"), "{}", later.stdout);
    assert!(!later.stdout.contains("Sync conflicts"), "{}", later.stdout);
}

#[test]
fn sync_a_prompt_without_a_query_still_raises_in_a_first_digest() {
    let rig = Rig::new("digest-sync-command", &[]);
    write(&rig.root, "gotcha-nix.md", &note("Nix", "Unrelated.\n"));
    open_json(
        &rig,
        serde_json::json!({ IDS[0]: conflict("gotcha-nix.md") }),
    );
    let run = rig.digest("abc", "/commit");
    assert_eq!(
        run.stdout,
        format!(
            "<!-- bilbo digest: 0 of 0 notes -->\n{}\n",
            waits(&rig, &["gotcha-nix.md"])
        )
    );
}

#[test]
fn sync_names_three_conflicts_and_counts_the_rest() {
    let rig = Rig::new("digest-sync-many", &[]);
    let names: Vec<String> = (1..=5).map(|n| format!("gotcha-nix-{n}.md")).collect();
    let mut entries = serde_json::Map::new();
    for (n, name) in names.iter().enumerate() {
        write(
            &rig.root,
            name,
            &note("Nix", "Unrelated.\n").replace(IDS[0], &id(n + 1)),
        );
        entries.insert(id(n + 1), conflict(name));
    }
    open_json(&rig, entries.into());
    let run = rig.digest("abc", PROMPT);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let line = run.stdout.lines().nth(1).unwrap().to_string();
    assert_eq!(run.stdout.lines().count(), 2, "{}", run.stdout);
    assert_eq!(
        line,
        format!(
            "{}, and 2 more (run bilbo check)",
            waits(&rig, &refs[..3]).trim_end_matches(" (run bilbo check)")
        )
    );
}

#[test]
fn sync_a_renamed_note_is_found_by_its_id() {
    let rig = Rig::new("digest-sync-renamed", &[]);
    sync_note(&rig, "gotcha-flakes.md", &id(1));
    open_json(
        &rig,
        serde_json::json!({ id(1): conflict("gotcha-nix.md") }),
    );
    let run = rig.digest("abc", PROMPT);
    assert!(
        run.stdout.contains(&format!(
            "/notes/gotcha-flakes.md:6 (gotcha, {CREATED}, conflict)"
        )),
        "{}",
        run.stdout
    );
    assert_eq!(
        run.stdout.lines().last().unwrap(),
        waits(&rig, &["gotcha-flakes.md"])
    );
}

#[test]
fn sync_an_entry_of_a_note_that_is_gone_is_ignored() {
    let rig = Rig::new("digest-sync-gone", &[]);
    write(&rig.root, "gotcha-slots.md", &note("Slots", "Unrelated.\n"));
    open_json(
        &rig,
        serde_json::json!({ id(9): conflict("gotcha-gone.md") }),
    );
    silent(&rig.digest("abc", PROMPT));
    assert!(!rig.sessions().join("abc").exists());
}

#[test]
fn sync_without_a_summary_labels_nothing() {
    let rig = Rig::new("digest-sync-none", &[]);
    hit(&rig, "gotcha-slots.md");
    let run = rig.digest("abc", PROMPT);
    assert!(!run.stdout.contains("conflict"), "{}", run.stdout);
    assert!(!run.stdout.contains("auto-merged"), "{}", run.stdout);
    assert!(!run.stdout.contains("Sync"), "{}", run.stdout);
}

#[test]
fn sync_an_unreadable_summary_is_noted_and_the_notes_still_show() {
    let rig = Rig::new("digest-sync-garbled", &[]);
    hit(&rig, "gotcha-slots.md");
    let dir = rig.root.join(".bilbo/sync");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("open.json"), "{not json").unwrap();
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.contains("gotcha-slots.md"), "{}", run.stdout);
    assert!(
        run.stderr.starts_with("bilbo: ") && run.stderr.contains("open.json"),
        "{}",
        run.stderr
    );
}

#[test]
fn sync_the_block_with_its_conflicts_line_stays_within_9000_bytes() {
    let rig = Rig::new("digest-sync-size", &[]);
    let body = "The embedder livelocks, a livelock on chunks. ".repeat(60);
    let mut entries = serde_json::Map::new();
    for n in 0..6 {
        write(
            &rig.root,
            &format!("gotcha-big-{n}.md"),
            &note("Big", &body).replace(IDS[0], &id(n + 1)),
        );
        entries.insert(id(n + 1), conflict(&format!("gotcha-big-{n}.md")));
    }
    open_json(&rig, entries.into());
    let run = rig.digest("abc", PROMPT);
    assert!(run.stdout.len() <= 9000, "{}", run.stdout.len());
    assert!(
        run.stdout.contains("Sync conflicts wait in"),
        "{}",
        run.stdout
    );
}
