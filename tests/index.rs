mod common;

use std::path::{Path, PathBuf};

use common::{
    Fake, IDS, Locked, Run, TempDir, bilbo, config, dead_url, guide, in_scope, library, note_text,
    snapshot, store, write,
};

struct Setup {
    dir: TempDir,
    root: PathBuf,
    config: PathBuf,
}

const TOKEN: &str = "sk-test-Zq8xW3vK9pL2mN7r";

/// A store and a config for the embedder at `url`, with `extra` config lines after the standard two.
fn setup_in(dir: TempDir, url: &str, extra: &[&str]) -> Setup {
    let root = store(&dir);
    let config = config_for(&dir, url, extra);
    Setup { dir, root, config }
}

fn setup(name: &str, fake: &Fake, extra: &[&str]) -> Setup {
    setup_in(TempDir::new(name), &fake.url, extra)
}

fn config_for(dir: &TempDir, url: &str, extra: &[&str]) -> PathBuf {
    let url = format!("embedder.url = {url}");
    let mut lines = vec![url.as_str(), "embedder.model = test-model"];
    lines.extend(extra);
    config(dir, &lines)
}

fn env(s: &Setup) -> Vec<(&'static str, String)> {
    vec![
        ("BILBO_HOME", s.root.to_str().unwrap().to_string()),
        ("BILBO_CONFIG", s.config.to_str().unwrap().to_string()),
        (
            "XDG_CACHE_HOME",
            s.dir.path().join("cache").to_str().unwrap().to_string(),
        ),
    ]
}

/// Runs `bilbo index`; a variable in `extra_env` replaces the standard one of the same name.
fn index(s: &Setup, extra_env: &[(&str, &str)], args: &[&str]) -> Run {
    let vars = env(s);
    let mut all: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
    all.extend_from_slice(extra_env);
    let mut full = vec!["index"];
    full.extend(args);
    bilbo(s.dir.path(), &all, &full)
}

/// The only `.vectors` file under `<dir>/cache/bilbo`.
fn cache_file(s: &Setup) -> PathBuf {
    let files = cache_files(&s.dir.path().join("cache"));
    assert_eq!(files.len(), 1, "{files:?}");
    files.into_iter().next().unwrap()
}

fn cache_files(cache: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(cache.join("bilbo")) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "vectors"))
        .collect();
    files.sort();
    files
}

/// The title is line 6, line 7 is blank and `body` starts on line 8.
fn note(title: &str, body: &str) -> String {
    format!("{}\n{body}", note_text(IDS[0], title))
}

/// Three passages: the title and body of the first note, its `## Layout` section and the second note's title.
fn three(s: &Setup) {
    let body = "Where notes live.\n## Layout\n\nOne flat folder.\n";
    write(&s.root, "decision-note-store.md", &note("Note store", body));
    write(&s.root, "plan-rollback.md", &note("Rollback", "Undo it.\n"));
}

/// `n` notes of one passage each, all different.
fn many(s: &Setup, n: usize) {
    for i in 0..n {
        let name = format!("plan-n{i:03}.md");
        write(
            &s.root,
            &name,
            &note(&format!("N{i}"), &format!("text {i}\n")),
        );
    }
}

fn ok(run: &Run) {
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

fn failed(run: &Run, code: i32) {
    assert_eq!(run.code, code, "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
}

fn authorization(request: &common::Request) -> Option<&str> {
    request
        .headers
        .iter()
        .find(|(name, _)| name == "authorization")
        .map(|(_, value)| value.as_str())
}

/// No 4-byte window of `secret` appears in `text`.
fn assert_no_window(text: &str, secret: &str) {
    for window in secret.as_bytes().windows(4) {
        let window = std::str::from_utf8(window).unwrap();
        assert!(!text.contains(window), "{window:?} in {text:?}");
    }
}

#[test]
fn first_run_embeds_every_passage() {
    let fake = Fake::start(4);
    let s = setup("index-first", &fake, &[]);
    three(&s);
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 0\n");
    assert_eq!(fake.inputs().len(), 3);
}

#[test]
fn second_run_sends_nothing() {
    let fake = Fake::start(4);
    let s = setup("index-second", &fake, &[]);
    three(&s);
    ok(&index(&s, &[], &[]));
    let file = cache_file(&s);
    let bytes = std::fs::read(&file).unwrap();
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    let sent = fake.requests().len();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 0, kept 3, dropped 0\n");
    assert_eq!(fake.requests().len(), sent);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        modified
    );
}

#[test]
fn edited_and_deleted_notes() {
    let fake = Fake::start(4);
    let s = setup("index-edited", &fake, &[]);
    let a = "## One\n\nfirst\n## Two\n\nsecond\n";
    write(&s.root, "plan-a.md", &note("A", a));
    write(&s.root, "plan-b.md", &note("B", "## Only\n\nbee\n"));
    let run = index(&s, &[], &[]);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 0\n");
    let a = "## One\n\nfirst\n## Two\n\nsecond changed\n";
    write(&s.root, "plan-a.md", &note("A", a));
    std::fs::remove_file(s.root.join("notes/plan-b.md")).unwrap();
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 1, kept 1, dropped 2\n");
    let last = fake.requests().pop().unwrap();
    assert_eq!(last.inputs, ["A > Two\nsecond changed"]);
}

#[test]
fn extra_argument_is_usage_error() {
    let fake = Fake::start(4);
    let s = setup("index-args", &fake, &[]);
    three(&s);
    let run = index(&s, &[], &["--rebuild"]);
    failed(&run, 2);
    assert!(
        run.stderr
            .starts_with("bilbo: unknown option '--rebuild'\n")
    );
    let run = index(&s, &[], &["foo"]);
    failed(&run, 2);
    assert!(run.stderr.starts_with("bilbo: unexpected argument 'foo'\n"));
    assert!(fake.requests().is_empty());
    assert!(!s.dir.path().join("cache").exists());
}

/// `config::is_local` calls `0.0.0.0` remote, and a connect to it reaches the fake's `127.0.0.1`
/// listener, so tests of remote-only behavior point the embedder there.
#[test]
fn zero_address() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let s = setup_in(TempDir::new("index-zero-address"), &url, &[]);
    write(&s.root, "plan-rollback.md", &note("Rollback", "Undo it.\n"));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 1, kept 0, dropped 0\n");
    assert_eq!(fake.inputs(), ["Rollback\nUndo it."]);
}

#[test]
fn input_carries_the_heading_path() {
    let fake = Fake::start(4);
    let s = setup("index-path", &fake, &[]);
    let body = "## Layout\n\nOne flat folder.\n";
    write(&s.root, "decision-note-store.md", &note("Note store", body));
    ok(&index(&s, &[], &[]));
    assert!(
        fake.inputs()
            .contains(&"Note store > Layout\nOne flat folder.".to_string()),
        "{:?}",
        fake.inputs()
    );
}

#[test]
fn requests_carry_model_and_batches_of_16() {
    let fake = Fake::start(4);
    let s = setup("index-batches", &fake, &[]);
    many(&s, 40);
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 40, kept 0, dropped 0\n");
    let requests = fake.requests();
    let sizes: Vec<usize> = requests.iter().map(|r| r.inputs.len()).collect();
    assert_eq!(sizes, [16, 16, 8]);
    for request in &requests {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/embeddings");
        assert_eq!(request.model, "test-model");
        assert_eq!(authorization(request), None);
    }
}

#[test]
fn token_from_file_and_variable() {
    let fake = Fake::start(4);

    let dir = TempDir::new("index-token-file");
    let token_file = dir.path().join("tok");
    std::fs::write(&token_file, "  file-token \n").unwrap();
    let line = format!("embedder.token_file = {}", token_file.display());
    let s = setup_in(dir, &fake.url, &[&line]);
    many(&s, 1);
    ok(&index(&s, &[], &[]));

    let dir = TempDir::new("index-token-tilde");
    let home = dir.path().join("h");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("tok"), "tilde-token\n").unwrap();
    let s = setup_in(dir, &fake.url, &["embedder.token_file = ~/tok"]);
    many(&s, 1);
    ok(&index(&s, &[("HOME", home.to_str().unwrap())], &[]));

    let s = setup(
        "index-token-var",
        &fake,
        &["embedder.token_env = EMBED_TOKEN"],
    );
    many(&s, 1);
    ok(&index(&s, &[("EMBED_TOKEN", " var-token\t")], &[]));

    let headers: Vec<Option<String>> = fake
        .requests()
        .iter()
        .map(|r| authorization(r).map(str::to_string))
        .collect();
    assert_eq!(
        headers,
        [
            Some("Bearer file-token".into()),
            Some("Bearer tilde-token".into()),
            Some("Bearer var-token".into()),
        ]
    );
}

#[test]
fn long_input_is_cut_at_4000_bytes() {
    let fake = Fake::start(4);
    let s = setup("index-long", &fake, &[]);
    let title = "T".repeat(26);
    let body = format!("## S\n\n{}\n", "x".repeat(3990));
    write(&s.root, "plan-long.md", &note(&title, &body));
    ok(&index(&s, &[], &[]));
    let prefix = format!("{title} > S\n");
    let long: Vec<String> = fake
        .inputs()
        .into_iter()
        .filter(|i| i.starts_with(&prefix))
        .collect();
    assert_eq!(long.len(), 1);
    assert_eq!(long[0].len(), 4000);
}

#[test]
fn malformed_answer_stores_nothing() {
    let fake = Fake::start(4);
    let s = setup("index-malformed", &fake, &[]);
    three(&s);
    fake.too_few();
    let run = index(&s, &[], &[]);
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: embedder {} answered 2 vectors for 3 inputs\n",
            fake.url
        )
    );
    assert!(cache_files(&s.dir.path().join("cache")).is_empty());
    fake.heal();
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 0\n");
}

#[test]
fn status_and_unreachable_name_the_url() {
    let fake = Fake::start(4);
    let s = setup("index-status", &fake, &[]);
    three(&s);
    fake.status(500);
    let run = index(&s, &[], &[]);
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!("bilbo: embedder {} answered 500\n", fake.url)
    );

    let dead = dead_url();
    let config = config_for(&s.dir, &dead, &[]);
    let run = bilbo(
        s.dir.path(),
        &[
            ("BILBO_HOME", s.root.to_str().unwrap()),
            ("BILBO_CONFIG", config.to_str().unwrap()),
            (
                "XDG_CACHE_HOME",
                s.dir.path().join("cache").to_str().unwrap(),
            ),
        ],
        &["index"],
    );
    failed(&run, 1);
    assert!(
        run.stderr
            .starts_with(&format!("bilbo: embedder {dead} unreachable: ")),
        "{}",
        run.stderr
    );
}

#[test]
fn no_embedder_configured() {
    let dir = TempDir::new("index-none");
    let root = store(&dir);
    let home = dir.path().join("home");
    let root_var = root.to_str().unwrap();
    let home_var = home.to_str().unwrap();
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", root_var), ("HOME", home_var)],
        &["index"],
    );
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no embedder configured; set embedder.url in {}/.config/bilbo/config\n",
            home.display()
        )
    );

    let comment = dir.path().join("config");
    std::fs::write(&comment, "# nothing here\n").unwrap();
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", root_var),
            ("HOME", home_var),
            ("BILBO_CONFIG", comment.to_str().unwrap()),
        ],
        &["index"],
    );
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no embedder configured; set embedder.url in {}\n",
            comment.display()
        )
    );

    let missing = dir.path().join("nostore");
    let run = bilbo(
        dir.path(),
        &[
            ("BILBO_HOME", missing.to_str().unwrap()),
            ("HOME", home_var),
        ],
        &["index"],
    );
    failed(&run, 1);
    assert!(run.stderr.contains("no embedder configured"));
    assert!(!missing.exists() && !home.exists());
}

#[test]
fn progress_survives_a_failure() {
    let fake = Fake::start(4);
    let s = setup("index-progress", &fake, &[]);
    many(&s, 100);
    fake.fail_after(4);
    let run = index(&s, &[], &[]);
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!("bilbo: embedder {} answered 500\n", fake.url)
    );
    assert_eq!(fake.requests().len(), 5);
    fake.heal();
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 36, kept 64, dropped 0\n");
}

#[test]
fn model_change_reembeds() {
    let fake = Fake::start(4);
    let s = setup("index-model", &fake, &[]);
    three(&s);
    ok(&index(&s, &[], &[]));
    config(
        &s.dir,
        &[
            &format!("embedder.url = {}", fake.url),
            "embedder.model = model-b",
        ],
    );
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 3\n");
    assert_eq!(fake.requests().last().unwrap().model, "model-b");
}

#[test]
fn two_stores_keep_their_own_cache() {
    let fake = Fake::start(4);
    let a = setup("index-two", &fake, &[]);
    three(&a);
    let b_root = a.dir.path().join("store-b");
    std::fs::create_dir_all(b_root.join("notes")).unwrap();
    write(
        &b_root,
        "plan-other.md",
        &note("Other", "Something else.\n"),
    );
    let b_home = b_root.to_str().unwrap();

    ok(&index(&a, &[], &[]));
    let cache = a.dir.path().join("cache");
    let a_file = cache_file(&a);
    let a_bytes = std::fs::read(&a_file).unwrap();

    let run = index(&a, &[("BILBO_HOME", b_home)], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 1, kept 0, dropped 0\n");
    assert_eq!(cache_files(&cache).len(), 2);
    assert_eq!(std::fs::read(&a_file).unwrap(), a_bytes);

    let run = index(&a, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 0, kept 3, dropped 0\n");
}

#[test]
fn deleted_cache_rebuilds() {
    let fake = Fake::start(4);
    let s = setup("index-deleted", &fake, &[]);
    three(&s);
    ok(&index(&s, &[], &[]));
    std::fs::remove_dir_all(s.dir.path().join("cache")).unwrap();
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 0\n");
}

#[test]
fn index_leaves_store_as_found() {
    let fake = Fake::start(4);
    let s = setup("index-snapshot", &fake, &[]);
    three(&s);
    write(&s.root, ".hidden", "x");
    write(&s.root, "bad name.md", "not a note");
    std::fs::create_dir_all(s.root.join("notes/sub")).unwrap();
    let before = snapshot(&s.root);
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 0\n");
    assert_eq!(snapshot(&s.root), before);
}

#[test]
fn missing_store_is_refused() {
    let fake = Fake::start(4);
    let s = setup("index-nostore", &fake, &[]);
    let missing = s.dir.path().join("nostore");
    let run = index(&s, &[("BILBO_HOME", missing.to_str().unwrap())], &[]);
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", missing.display())
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn empty_store_is_fine() {
    let fake = Fake::start(4);
    let s = setup("index-empty", &fake, &[]);
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 0, kept 0, dropped 0\n");
    assert!(fake.requests().is_empty());

    three(&s);
    ok(&index(&s, &[], &[]));
    for name in ["decision-note-store.md", "plan-rollback.md"] {
        std::fs::remove_file(s.root.join("notes").join(name)).unwrap();
    }
    let sent = fake.requests().len();
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 0, kept 0, dropped 3\n");
    assert_eq!(fake.requests().len(), sent);
}

#[test]
fn identical_passages_share_a_vector() {
    let fake = Fake::start(4);
    let s = setup("index-identical", &fake, &[]);
    write(&s.root, "plan-a.md", &note("Same", "Body.\n"));
    write(&s.root, "plan-b.md", &note("Same", "Body.\n"));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 1, kept 0, dropped 0\n");
    assert_eq!(fake.inputs(), ["Same\nBody."]);
}

#[test]
fn empty_token_variable() {
    let fake = Fake::start(4);
    let s = setup(
        "index-empty-var",
        &fake,
        &["embedder.token_env = EMBED_TOKEN"],
    );
    three(&s);
    let run = index(&s, &[("EMBED_TOKEN", "")], &[]);
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        "bilbo: embedder token variable EMBED_TOKEN is empty\n"
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn missing_token_file() {
    let fake = Fake::start(4);
    let dir = TempDir::new("index-missing-token");
    let token = dir.path().join("nope");
    let line = format!("embedder.token_file = {}", token.display());
    let s = setup_in(dir, &fake.url, &[&line]);
    three(&s);
    let run = index(&s, &[], &[]);
    failed(&run, 1);
    assert!(
        run.stderr.starts_with(&format!(
            "bilbo: cannot read embedder token file {}: ",
            token.display()
        )),
        "{}",
        run.stderr
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn rejected_token_is_not_echoed() {
    let fake = Fake::start(4);
    let dir = TempDir::new("index-rejected");
    let token = dir.path().join("tok");
    std::fs::write(&token, format!("{TOKEN}\n")).unwrap();
    let line = format!("embedder.token_file = {}", token.display());
    let s = setup_in(dir, &fake.url, &[&line]);
    three(&s);
    fake.status(401);
    let run = index(&s, &[], &[]);
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!("bilbo: embedder {} answered 401\n", fake.url)
    );
    assert_no_window(&format!("{}{}", run.stdout, run.stderr), "Zq8xW3vK9pL2mN7r");
    let sent = fake.requests();
    assert_eq!(
        authorization(&sent[0]),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

#[test]
fn cache_file_layout() {
    let fake = Fake::start(4);
    fake.vector("", &[3.0, 4.0, 0.0, 0.0]);
    let s = setup("index-layout", &fake, &[]);
    three(&s);
    ok(&index(&s, &[], &[]));
    let bytes = std::fs::read(cache_file(&s)).unwrap();
    let mut head = b"BILBOVEC1\n".to_vec();
    head.extend_from_slice(&10u32.to_le_bytes());
    head.extend_from_slice(b"test-model");
    head.extend_from_slice(&4u32.to_le_bytes());
    head.extend_from_slice(&3u32.to_le_bytes());
    assert!(bytes.starts_with(&head));
    assert_eq!(bytes.len(), 10 + 4 + 10 + 4 + 4 + 3 * 24);
    let first = &bytes[head.len() + 8..head.len() + 24];
    let vector: Vec<f32> = first
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    for (got, want) in vector.iter().zip([0.6, 0.8, 0.0, 0.0]) {
        assert!((got - want).abs() < 1e-6, "{vector:?}");
    }
}

#[test]
fn no_cache_folder() {
    let fake = Fake::start(4);
    let s = setup("index-nocache", &fake, &[]);
    three(&s);
    let run = bilbo(
        s.dir.path(),
        &[
            ("BILBO_HOME", s.root.to_str().unwrap()),
            ("BILBO_CONFIG", s.config.to_str().unwrap()),
        ],
        &["index"],
    );
    failed(&run, 2);
    assert_eq!(
        run.stderr,
        "bilbo: cannot find the cache folder: set XDG_CACHE_HOME, or HOME, to an absolute path\n"
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn config_errors_exit_2() {
    let fake = Fake::start(4);
    let s = setup("index-config", &fake, &[]);
    three(&s);
    let path = s.config.display().to_string();
    let url = format!("embedder.url = {}", fake.url);
    let keys = "keys: embedder.url, embedder.model, embedder.token_file, embedder.token_env, embedder.query_prefix, embedder.min_similarity, digest.enable, digest.min_similarity, digest.log, history.keep_days, sync.poll_seconds, sync.stale_days, scope.<name>.sync, scope.<name>.embedder, scope.<name>.paths, scope.<name>.marks, scope.default";
    let cases: Vec<(Vec<&str>, String)> = vec![
        (
            vec!["# c", "embedder.model = x", "embeder.url = http://h"],
            format!("bilbo: {path}:3: unknown key 'embeder.url'; {keys}\n"),
        ),
        (
            vec![&url],
            format!("bilbo: {path}: embedder.url is set but embedder.model is not\n"),
        ),
        (
            vec![
                &url,
                "embedder.model = m",
                "embedder.token_file = /x",
                "embedder.token_env = Y",
            ],
            format!("bilbo: {path}: set embedder.token_file or embedder.token_env, not both\n"),
        ),
        (
            vec![&url, "embedder.model = m", "embedder.min_similarity = 1.5"],
            format!(
                "bilbo: {path}:3: embedder.min_similarity must be a number from 0 to 1, got '1.5'\n"
            ),
        ),
        (
            vec![&url, "embedder.model ="],
            format!("bilbo: {path}:2: embedder.model needs a value\n"),
        ),
        (
            vec![&url, "embedder.model = m", "scope.work.embedder = remote"],
            String::new(),
        ),
    ];
    for (lines, stderr) in cases {
        common::config(&s.dir, &lines);
        let run = index(&s, &[], &[]);
        failed(&run, 2);
        if stderr.is_empty() {
            assert!(run.stderr.contains("scope.work.embedder"), "{}", run.stderr);
        } else {
            assert_eq!(run.stderr, stderr);
        }
    }

    common::config(
        &s.dir,
        &[
            "embedder.url = http://u:sekrit@embedder.example:8081",
            "embedder.model = m",
        ],
    );
    let run = index(&s, &[], &[]);
    failed(&run, 2);
    assert!(run.stderr.contains(&format!("{path}:1")), "{}", run.stderr);
    assert!(run.stderr.contains("embedder.url"), "{}", run.stderr);
    assert_no_window(&run.stderr, "sekrit");

    let missing = s.dir.path().join("no-such-config");
    let run = index(&s, &[("BILBO_CONFIG", missing.to_str().unwrap())], &[]);
    failed(&run, 2);
    assert!(
        run.stderr
            .starts_with(&format!("bilbo: cannot read {}: ", missing.display())),
        "{}",
        run.stderr
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn dimension_change_is_refused() {
    let fake = Fake::start(4);
    let s = setup("index-dims", &fake, &[]);
    three(&s);
    ok(&index(&s, &[], &[]));
    let before = std::fs::read(cache_file(&s)).unwrap();

    let narrow = Fake::start(3);
    config_for(&s.dir, &narrow.url, &[]);
    write(&s.root, "plan-new.md", &note("New", "Fresh.\n"));
    let run = index(&s, &[], &[]);
    failed(&run, 1);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: embedder {} answered vectors of 3 dimensions; the cache holds 4; delete {} and run bilbo index again\n",
            narrow.url,
            cache_file(&s).display()
        )
    );
    assert_eq!(std::fs::read(cache_file(&s)).unwrap(), before);
}

#[test]
fn a_heading_without_text_is_not_sent() {
    let fake = Fake::start(4);
    let s = setup("index-textless", &fake, &[]);
    write(&s.root, "plan-t.md", &note("T", "## A\n\nsome text\n"));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 1, kept 0, dropped 0\n");
    assert_eq!(fake.inputs(), ["T > A\nsome text"]);
}

#[test]
fn old_vectors_of_textless_passages_are_dropped() {
    let fake = Fake::start(4);
    let s = setup("index-textless-old", &fake, &[]);
    write(&s.root, "plan-t.md", &note("T", "intro\n"));
    ok(&index(&s, &[], &[]));
    write(&s.root, "plan-t.md", &note("T", ""));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 0, kept 0, dropped 1\n");
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn digest_similarity_out_of_range() {
    let fake = Fake::start(4);
    let s = setup("index-digest-similarity", &fake, &[]);
    three(&s);
    common::config(&s.dir, &["digest.min_similarity = 1.5"]);
    let run = index(&s, &[], &[]);
    failed(&run, 2);
    assert!(
        run.stderr
            .contains(&format!("{}:1: digest.min_similarity", s.config.display())),
        "{}",
        run.stderr
    );
    assert!(fake.requests().is_empty());
}

#[test]
fn index_embeds_no_library_text() {
    let fake = Fake::start(4);
    let s = setup("index-no-library", &fake, &[]);
    write(&s.root, "plan-rollback.md", &note("Rollback", "Undo it.\n"));
    for n in 0..20 {
        library(
            &s.root,
            "go",
            &format!("source-{n:02}"),
            "# Goroutines\n\nA goroutine leak.\n",
        );
    }
    guide(
        &s.root,
        "go",
        "About go.",
        &[("source-00", "goroutine notes")],
    );
    let before = snapshot(&s.root.join("library"));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 1, kept 0, dropped 0\n");
    let inputs = fake.inputs();
    assert_eq!(inputs.len(), 1, "{inputs:?}");
    assert!(inputs[0].contains("Undo it."), "{inputs:?}");
    assert!(
        inputs.iter().all(|i| !i.contains("goroutine")),
        "{inputs:?}"
    );
    assert_eq!(snapshot(&s.root.join("library")), before);

    let locked = Locked::new(&s.root.join("library"), 0o755);
    let again = index(&s, &[], &[]);
    drop(locked);
    ok(&again);
    assert_eq!(again.stdout, "embedded 0, kept 1, dropped 0\n");
    assert_eq!(fake.inputs().len(), 1);
}

/// The vectors in the cache file, 4 dimensions each.
fn cached(s: &Setup) -> usize {
    let bytes = std::fs::read(cache_file(s)).unwrap();
    (bytes.len() - (10 + 4 + 10 + 4 + 4)) / 24
}

fn withheld_line(url: &str, n: usize) -> String {
    format!(
        "bilbo: withheld {n} passages from {url}: their scope allows only a loopback embedder\n"
    )
}

const LOCAL_WORK: &[&str] = &[
    "scope.work.embedder = local",
    "scope.personal.embedder = any",
];

#[test]
fn a_local_scope_withholds_its_passages_from_a_remote_embedder() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let s = setup_in(TempDir::new("index-withheld"), &url, LOCAL_WORK);
    let body = "Where notes live.\n## Layout\n\nOne flat folder.\n";
    write(
        &s.root,
        "plan-a.md",
        &in_scope(&note("Secret", body), "work"),
    );
    write(
        &s.root,
        "plan-b.md",
        &in_scope(&note("Open", "Free text.\n"), "personal"),
    );
    let run = index(&s, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "embedded 1, kept 0, dropped 0\n");
    assert_eq!(run.stderr, withheld_line(&url, 2));
    assert_eq!(fake.inputs(), ["Open\nFree text."]);
    assert_eq!(cached(&s), 1);
}

#[test]
fn unassigned_notes_take_the_strictest_rule() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let s = setup_in(TempDir::new("index-withheld-unassigned"), &url, LOCAL_WORK);
    write(&s.root, "plan-a.md", &note("Loose", "No scope.\n"));
    let run = index(&s, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "embedded 0, kept 0, dropped 0\n");
    assert_eq!(run.stderr, withheld_line(&url, 1));
    assert!(fake.requests().is_empty());
}

#[test]
fn the_withheld_count_is_distinct_inputs() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let s = setup_in(TempDir::new("index-withheld-distinct"), &url, LOCAL_WORK);
    write(
        &s.root,
        "plan-a.md",
        &in_scope(&note("Same", "Twice.\n"), "work"),
    );
    write(
        &s.root,
        "plan-b.md",
        &in_scope(&note("Same", "Twice.\n"), "work"),
    );
    let run = index(&s, &[], &[]);
    assert_eq!(run.stderr, withheld_line(&url, 1));
}

#[test]
fn text_shared_with_an_any_note_is_sent_once() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let s = setup_in(TempDir::new("index-withheld-shared"), &url, LOCAL_WORK);
    write(
        &s.root,
        "plan-a.md",
        &in_scope(&note("Deploy", "Push it.\n"), "work"),
    );
    write(
        &s.root,
        "plan-b.md",
        &in_scope(&note("Deploy", "Push it.\n"), "personal"),
    );
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 1, kept 0, dropped 0\n");
    assert_eq!(fake.inputs(), ["Deploy\nPush it."]);
}

#[test]
fn a_loopback_embedder_gets_everything() {
    let fake = Fake::start(4);
    let s = setup("index-withheld-loopback", &fake, LOCAL_WORK);
    write(
        &s.root,
        "plan-a.md",
        &in_scope(&note("One", "First.\n"), "work"),
    );
    write(
        &s.root,
        "plan-b.md",
        &in_scope(&note("Two", "Second.\n"), "personal"),
    );
    write(&s.root, "plan-c.md", &note("Three", "Third.\n"));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 0\n");
    assert_eq!(fake.inputs().len(), 3);
}

#[test]
fn no_scope_asking_for_local_withholds_nothing() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let s = setup_in(
        TempDir::new("index-withheld-none"),
        &url,
        &["scope.work.embedder = any"],
    );
    write(
        &s.root,
        "plan-a.md",
        &in_scope(&note("One", "First.\n"), "work"),
    );
    write(&s.root, "plan-b.md", &note("Two", "Unassigned.\n"));
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 2, kept 0, dropped 0\n");
}

#[test]
fn a_note_moving_into_a_local_scope_loses_its_vectors() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let dir = TempDir::new("index-withheld-moved");
    let s = setup_in(dir, &url, &["scope.work.embedder = any"]);
    let body = "Where notes live.\n## Layout\n\nOne flat folder.\n";
    write(
        &s.root,
        "plan-a.md",
        &in_scope(&note("Moved", body), "work"),
    );
    let run = index(&s, &[], &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 2, kept 0, dropped 0\n");
    assert_eq!(cached(&s), 2);

    let path = config_for(&s.dir, &url, &["scope.work.embedder = local"]);
    assert_eq!(path, s.config);
    let sent = fake.requests().len();
    let run = index(&s, &[], &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "embedded 0, kept 0, dropped 2\n");
    assert_eq!(run.stderr, withheld_line(&url, 2));
    assert_eq!(fake.requests().len(), sent);
    assert_eq!(cached(&s), 0);
}

#[test]
fn the_withheld_line_comes_before_an_embedder_error() {
    let fake = Fake::start(4);
    let url = format!("http://0.0.0.0:{}", fake.port());
    let s = setup_in(TempDir::new("index-withheld-error"), &url, LOCAL_WORK);
    write(
        &s.root,
        "plan-a.md",
        &in_scope(&note("Secret", "Hidden.\n"), "work"),
    );
    write(
        &s.root,
        "plan-b.md",
        &in_scope(&note("Open", "Free.\n"), "personal"),
    );
    fake.status(500);
    let run = index(&s, &[], &[]);
    failed(&run, 1);
    assert!(
        run.stderr.starts_with(&withheld_line(&url, 1)),
        "{}",
        run.stderr
    );
    assert_eq!(run.stderr.lines().count(), 2, "{}", run.stderr);
}

#[test]
fn a_proxy_variable_does_not_reroute_a_loopback_embedder() {
    let fake = Fake::start(4);
    let proxy = Fake::start(4);
    let s = setup("index-proxy", &fake, &[]);
    three(&s);
    let via = proxy.url.as_str();
    let vars = [
        ("HTTP_PROXY", via),
        ("http_proxy", via),
        ("HTTPS_PROXY", via),
        ("https_proxy", via),
        ("ALL_PROXY", via),
        ("all_proxy", via),
    ];
    let run = index(&s, &vars, &[]);
    ok(&run);
    assert_eq!(run.stdout, "embedded 3, kept 0, dropped 0\n");
    assert_eq!(fake.inputs().len(), 3);
    assert!(proxy.requests().is_empty(), "{:?}", proxy.requests().len());
}

#[test]
fn a_terminal_gets_the_marked_line() {
    let fake = Fake::start(4);
    let s = setup("index-tty", &fake, &[]);
    three(&s);
    let mut vars: Vec<(&str, &str)> = vec![("NO_COLOR", "1"), ("LANG", "C.UTF-8")];
    let base = env(&s);
    vars.extend(base.iter().map(|(k, v)| (*k, v.as_str())));
    let run = common::bilbo_tty(s.dir.path(), &vars, &["index"], 100);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "◆  Embedded 3 passages · kept 0 · dropped 0\n");
}
