mod common;

use common::{TempDir, bilbo, bilbo_input};

/// The verbs in the order of the overview and of a usage error's `verbs:` line.
const VERBS: [&str; 16] = [
    "new", "recall", "check", "history", "restore", "library", "cite", "scope", "sync", "device",
    "pair", "relay", "setup", "index", "watch", "digest",
];

/// What a usage error prints after its message when no verb was run.
fn top_usage() -> String {
    format!(
        "bilbo: usage: bilbo <verb> [<args>]...\nbilbo: verbs: {}\nbilbo: see 'bilbo --help'\n",
        VERBS.join(", ")
    )
}

/// The parser files of each verb, which hold every option it accepts.
const PARSERS: [(&str, &[&str]); 16] = [
    ("new", &["src/note/new.rs"]),
    ("recall", &["src/search/recall.rs"]),
    ("check", &["src/check.rs"]),
    ("history", &["src/note/history.rs"]),
    ("restore", &["src/note/restore.rs"]),
    ("library", &["src/library/cli/mod.rs"]),
    ("cite", &["src/citation/cite.rs"]),
    ("scope", &["src/note/scope.rs"]),
    ("sync", &["src/sync/cli.rs"]),
    ("device", &["src/identity/device.rs"]),
    ("pair", &["src/identity/pair/mod.rs"]),
    ("relay", &["src/relay/mod.rs"]),
    ("setup", &["src/setup/flags.rs"]),
    ("index", &["src/search/index.rs"]),
    ("watch", &["src/note/watch.rs"]),
    ("digest", &["src/search/digest.rs"]),
];

fn source(rel: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// `text` before the `#[cfg(test)]` line that opens `mod tests`.
fn production(text: &str) -> &str {
    match text.find("#[cfg(test)]\nmod tests") {
        Some(at) => &text[..at],
        None => text,
    }
}

/// Every string literal of `code` that is a whole long option, `"--name"` or `"--name="`, as `--name`.
fn options(code: &str) -> std::collections::BTreeSet<String> {
    code.match_indices("\"--")
        .filter_map(|(at, _)| {
            let literal = code[at + 1..].split('"').next()?;
            let name = literal.strip_suffix('=').unwrap_or(literal);
            let word = name.strip_prefix("--")?;
            (word.starts_with(|c: char| c.is_ascii_lowercase())
                && word.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
            .then(|| name.to_string())
        })
        .collect()
}

#[test]
fn no_arguments_is_usage_error() {
    let dir = TempDir::new("cli-none");
    let run = bilbo(dir.path(), &[], &[]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert_eq!(run.stderr, format!("bilbo: missing verb\n{}", top_usage()));
}

#[test]
fn unknown_verb_is_usage_error() {
    let dir = TempDir::new("cli-verb");
    let home = dir.path().join("home");
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", home.to_str().unwrap())],
        &["frobnicate"],
    );
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: unknown verb 'frobnicate'\n{}", top_usage())
    );
    assert!(!home.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn digest_is_a_verb() {
    let dir = TempDir::new("cli-digest");
    let home = dir.path().join("home");
    let run = bilbo_input(
        dir.path(),
        &[("BILBO_HOME", home.to_str().unwrap())],
        &["digest"],
        "not json",
    );
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
    assert_eq!(run.stderr, "bilbo: the hook input is not a JSON object\n");
}

#[test]
fn digest_exits_0_on_an_unknown_option() {
    let dir = TempDir::new("cli-digest-option");
    let home = dir.path().join("home");
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", home.to_str().unwrap())],
        &["digest", "--verbose"],
    );
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
    assert_eq!(run.stderr.lines().count(), 1);
    assert!(run.stderr.starts_with("bilbo: ") && run.stderr.contains("--verbose"));
}

#[test]
fn help_goes_to_stdout() {
    let dir = TempDir::new("cli-help");
    let overview = bilbo(dir.path(), &[], &["--help"]).stdout;
    assert!(overview.starts_with("bilbo keeps durable memory for coding agents"));
    assert!(overview.contains("\nUsage: bilbo <verb> [<args>]...\n"));
    for args in [&["--help"][..], &["-h"], &["help"], &["help", "-h"]] {
        let run = bilbo(dir.path(), &[], args);
        assert_eq!(run.code, 0, "{args:?}");
        assert_eq!(run.stdout, overview, "{args:?}");
        assert!(run.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn help_after_a_verb_prints_its_page() {
    let dir = TempDir::new("cli-help-verb");
    for verb in VERBS {
        let page = bilbo(dir.path(), &[], &[verb, "--help"]);
        assert_eq!(page.code, 0, "{verb}");
        assert!(page.stderr.is_empty(), "{verb}");
        assert!(
            page.stdout.starts_with(&format!("bilbo {verb}: ")),
            "{verb}"
        );
        assert!(page.stdout.contains(&format!("\n  bilbo {verb}")), "{verb}");
        for args in [[verb, "-h"], ["help", verb]] {
            let run = bilbo(dir.path(), &[], &args);
            assert_eq!(run.code, 0, "{args:?}");
            assert_eq!(run.stdout, page.stdout, "{args:?}");
        }
    }
    let recall = bilbo(dir.path(), &[], &["recall", "--help"]).stdout;
    assert!(recall.contains("bilbo recall <query>") && !recall.contains("bilbo new <kind>"));
}

#[test]
fn a_subcommand_help_is_its_verbs_page() {
    let dir = TempDir::new("cli-help-sub");
    let library = bilbo(dir.path(), &[], &["library", "--help"]).stdout;
    for args in [["library", "land", "-h"], ["library", "show", "--help"]] {
        assert_eq!(bilbo(dir.path(), &[], &args).stdout, library, "{args:?}");
    }
}

#[test]
fn help_for_an_unknown_verb_is_a_usage_error() {
    let dir = TempDir::new("cli-help-unknown");
    for args in [&["help", "frobnicate"][..], &["frobnicate", "--help"]] {
        let run = bilbo(dir.path(), &[], args);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stdout.is_empty(), "{args:?}");
        assert_eq!(
            run.stderr,
            format!("bilbo: unknown verb 'frobnicate'\n{}", top_usage())
        );
    }
    let run = bilbo(dir.path(), &[], &["help", "recall", "now"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        format!("bilbo: unexpected argument 'now'\n{}", top_usage())
    );
}

#[test]
fn piped_help_holds_no_escape_byte() {
    let dir = TempDir::new("cli-help-plain");
    let env = [("TERM", "xterm-256color")];
    let mut runs = vec![bilbo(dir.path(), &env, &["--help"])];
    for verb in VERBS {
        runs.push(bilbo(dir.path(), &env, &[verb, "--help"]));
    }
    for run in runs {
        assert!(!run.stdout.contains('\x1b'), "{}", run.stdout);
    }
}

#[test]
fn a_usage_error_is_the_reason_the_forms_and_the_page() {
    let dir = TempDir::new("cli-usage-shape");
    let run = bilbo(dir.path(), &[], &["recall", "--bogus"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        "bilbo: unknown option '--bogus'\n\
         bilbo: usage: bilbo recall <query>... [--kind <kind>]... [--limit <n>]\n\
         bilbo:        bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]\n\
         bilbo: see 'bilbo recall --help'\n"
    );
    let run = bilbo(dir.path(), &[], &["library", "land"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        "bilbo: missing <stage>\n\
         bilbo: usage: bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]\n\
         bilbo: see 'bilbo library --help'\n"
    );
}

#[test]
fn an_unknown_verb_near_a_verb_gets_a_suggestion() {
    let dir = TempDir::new("cli-suggest");
    let run = bilbo(dir.path(), &[], &["recal", "x"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: unknown verb 'recal'; did you mean 'recall'?\n{}",
            top_usage()
        )
    );
    let run = bilbo(dir.path(), &[], &["zzzzzz"]);
    assert_eq!(
        run.stderr,
        format!("bilbo: unknown verb 'zzzzzz'\n{}", top_usage())
    );
}

#[test]
fn every_dispatched_verb_is_in_the_overview() {
    let main = source("src/main.rs");
    let dispatched: std::collections::BTreeSet<&str> = production(&main)
        .split("Some(\"")
        .skip(1)
        .filter_map(|rest| rest.split_once("\") =>").map(|(verb, _)| verb))
        .filter(|verb| !verb.starts_with('-'))
        .collect();
    assert_eq!(dispatched, VERBS.into_iter().collect());
    let dir = TempDir::new("cli-overview-verbs");
    let overview = bilbo(dir.path(), &[], &["--help"]).stdout;
    let groups = overview.split("\nExamples:").next().unwrap();
    let listed: Vec<&str> = groups
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|line| line.split_once("  ").map(|(verb, _)| verb))
        .collect();
    assert_eq!(listed, VERBS);
}

#[test]
fn every_option_a_parser_accepts_is_on_its_page() {
    let dir = TempDir::new("cli-help-options");
    assert_eq!(PARSERS.map(|(verb, _)| verb), VERBS);
    for (verb, files) in PARSERS {
        let page = bilbo(dir.path(), &[], &[verb, "--help"]).stdout;
        for file in files {
            for option in options(production(&source(file))) {
                let shown = page.match_indices(&option).any(|(at, _)| {
                    !page[at + option.len()..]
                        .starts_with(|c: char| c.is_ascii_lowercase() || c == '-')
                });
                assert!(shown, "{verb}: {option} from {file} is not on its page");
            }
        }
    }
    for (file, option) in [
        ("src/note/new.rs", "--title"),
        ("src/library/cli/mod.rs", "--depth"),
        ("src/setup/flags.rs", "--index-every"),
    ] {
        assert!(
            options(production(&source(file))).contains(option),
            "the scan no longer sees {option} in {file}"
        );
    }
}

#[test]
fn every_page_links_a_section_of_the_commands_reference() {
    let commands = source("docs/reference/commands.md");
    assert!(commands.starts_with("# Commands\n"));
    let dir = TempDir::new("cli-help-docs");
    for verb in VERBS {
        let page = bilbo(dir.path(), &[], &[verb, "--help"]).stdout;
        let link = format!(
            "Docs: {}/wiki/Commands#{verb}\n",
            env!("CARGO_PKG_REPOSITORY")
        );
        assert!(page.ends_with(&link), "{verb}");
        assert!(commands.contains(&format!("\n## {verb}\n")), "{verb}");
    }
}

#[test]
fn unknown_top_level_option_is_usage_error() {
    let dir = TempDir::new("cli-option");
    let run = bilbo(dir.path(), &[], &["--verbose"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with("bilbo: unknown option '--verbose'\n")
    );
}

#[test]
fn version_goes_to_stdout() {
    let dir = TempDir::new("cli-version");
    let run = bilbo(dir.path(), &[], &["--version"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, format!("bilbo {}\n", env!("CARGO_PKG_VERSION")));
    assert!(run.stderr.is_empty());
}

#[test]
fn version_is_not_a_verb_option() {
    let dir = TempDir::new("cli-version-verb");
    let run = bilbo(dir.path(), &[], &["check", "--version"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: unknown option '--version'\n"));
}

#[test]
fn version_with_more_arguments_is_usage_error() {
    let dir = TempDir::new("cli-version-extra");
    let run = bilbo(dir.path(), &[], &["--version", "now"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: unexpected argument 'now'\n{}", top_usage())
    );
}

fn home(dir: &TempDir) -> String {
    dir.path().join("store").to_str().unwrap().to_string()
}

#[test]
fn check_verb_runs() {
    let dir = TempDir::new("cli-check");
    let home = home(&dir);
    std::fs::create_dir_all(dir.path().join("store/notes")).unwrap();
    let run = bilbo(dir.path(), &[("BILBO_HOME", &home)], &["check"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.is_empty());
}

#[test]
fn no_recall_hits_exits_1() {
    let dir = TempDir::new("cli-recall-empty");
    let home = home(&dir);
    std::fs::create_dir_all(dir.path().join("store/notes")).unwrap();
    let run = bilbo(dir.path(), &[("BILBO_HOME", &home)], &["recall", "wumpus"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(run.stderr, "bilbo: no notes match\n");
}

#[test]
fn unknown_option_is_not_help() {
    let dir = TempDir::new("cli-unknown-option");
    let run = bilbo(dir.path(), &[], &["check", "--verbose"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: unknown option '--verbose'\n"));
}

#[test]
fn success_exits_0() {
    let dir = TempDir::new("cli-success");
    let home = home(&dir);
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", &home)],
        &["new", "plan", "release-steps"],
    );
    assert_eq!(run.code, 0);
}

#[test]
fn refusal_exits_1() {
    let dir = TempDir::new("cli-refusal");
    let home = home(&dir);
    let env = [("BILBO_HOME", home.as_str())];
    assert_eq!(
        bilbo(dir.path(), &env, &["new", "plan", "release-steps"]).code,
        0
    );
    assert_eq!(
        bilbo(dir.path(), &env, &["new", "plan", "release-steps"]).code,
        1
    );
}

#[test]
fn missing_topic_exits_2() {
    let dir = TempDir::new("cli-missing-topic");
    let run = bilbo(dir.path(), &[], &["new", "plan"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.starts_with("bilbo: missing <topic>\n"));
}

#[test]
fn failure_leaves_stdout_empty() {
    let dir = TempDir::new("cli-failure-stdout");
    let home = home(&dir);
    let env = [("BILBO_HOME", home.as_str())];
    bilbo(dir.path(), &env, &["new", "plan", "release"]);
    let run = bilbo(dir.path(), &env, &["new", "decision", "release"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.starts_with("bilbo: "));
}

#[test]
fn relative_bilbo_home_is_refused() {
    let dir = TempDir::new("cli-relative-home");
    for args in [
        &["new", "plan", "x"][..],
        &["check"][..],
        &["recall", "rollback"][..],
    ] {
        let run = bilbo(dir.path(), &[("BILBO_HOME", "store")], args);
        assert_eq!(run.code, 2);
        assert!(run.stdout.is_empty());
        assert_eq!(
            run.stderr,
            "bilbo: BILBO_HOME must be an absolute path, got 'store'\n"
        );
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn every_stderr_line_of_a_multi_line_argument_is_prefixed() {
    let dir = TempDir::new("cli-newline-arg");
    let run = bilbo(dir.path(), &[], &["foo\nbar"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr
            .starts_with("bilbo: unknown verb 'foo\nbilbo: bar'\n")
    );
}

#[test]
fn non_utf8_argument_is_usage_error() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new("cli-non-utf8");
    let arg = std::ffi::OsStr::from_bytes(b"\xff\xfe");
    let run = common::bilbo_os(dir.path(), &[], &[arg]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with("bilbo: arguments must be valid UTF-8\n")
    );
}

#[test]
fn fake_embedder_answers() {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn post(fake: &common::Fake, body: &str) -> String {
        let mut stream = TcpStream::connect(fake.url.trim_start_matches("http://")).unwrap();
        write!(
            stream,
            "POST /v1/embeddings HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    let fake = common::Fake::start(4);
    fake.vector("alpha", &[3.0, 4.0, 0.0, 0.0]);
    let answer = post(&fake, r#"{"model":"m","input":["alpha one","beta"]}"#);
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert_eq!(answer.matches(r#""embedding":["#).count(), 2, "{answer}");
    assert!(answer.contains("[3,4,0,0]"), "{answer}");
    assert!(answer.contains("[0,0,0,1]"), "{answer}");
    assert_eq!(fake.inputs(), ["alpha one", "beta"]);

    fake.status(401);
    assert!(post(&fake, "{}").starts_with("HTTP/1.1 401 Unauthorized\r\n"));
    drop(fake);
}

#[test]
fn check_and_new_read_the_config() {
    let dir = TempDir::new("cli-read-config");
    let home = home(&dir);
    std::fs::create_dir_all(dir.path().join("store/notes")).unwrap();
    let missing = dir.path().join("no-such-config");
    let env = [
        ("BILBO_HOME", home.as_str()),
        ("BILBO_CONFIG", missing.to_str().unwrap()),
    ];
    for args in [&["check"][..], &["new", "plan", "x"]] {
        let run = bilbo(dir.path(), &env, args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty(), "{args:?}");
        assert!(
            run.stderr.contains(missing.to_str().unwrap()),
            "{}",
            run.stderr
        );
    }
    assert_eq!(
        std::fs::read_dir(dir.path().join("store/notes"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn fake_embedder_recovers_from_a_stall() {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    fn connect(fake: &common::Fake, path: &str) -> TcpStream {
        let mut stream = TcpStream::connect(fake.url.trim_start_matches("http://")).unwrap();
        let body = r#"{"model":"m","input":["a"]}"#;
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        stream
    }

    let fake = common::Fake::start(4);
    fake.stall();
    let mut stalled = connect(&fake, "/v1/embeddings");
    stalled
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    assert!(stalled.read(&mut [0u8; 1]).is_err());
    drop(stalled);
    fake.heal();
    let mut answer = String::new();
    connect(&fake, "/v1/embeddings")
        .read_to_string(&mut answer)
        .unwrap();
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");

    let mut answer = String::new();
    connect(&fake, "/v2/embeddings")
        .read_to_string(&mut answer)
        .unwrap();
    assert!(answer.starts_with("HTTP/1.1 404 Not Found\r\n"), "{answer}");
    assert_eq!(fake.requests().len(), 3);

    fake.stall();
    let _held = connect(&fake, "/v1/embeddings");
    std::thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    drop(fake);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn setup_is_a_verb() {
    let dir = TempDir::new("cli-setup");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let run = bilbo(
        dir.path(),
        &[("HOME", home.to_str().unwrap())],
        &["setup", "--yes", "--no-plugin", "--no-watch"],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.is_empty());
    assert!(run.stdout.starts_with("store created: "));
}

#[test]
fn watch_is_a_verb() {
    let dir = TempDir::new("cli-watch");
    let run = bilbo(dir.path(), &[], &["watch", "--now"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.starts_with("bilbo: unknown option '--now'\n"));
}

#[test]
fn history_is_a_verb() {
    let dir = TempDir::new("cli-history");
    let home = dir.path().join("home");
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", home.to_str().unwrap())],
        &["history", "release"],
    );
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", home.display())
    );
    assert!(!home.exists());
}

#[test]
fn restore_is_a_verb() {
    let dir = TempDir::new("cli-restore");
    let home = dir.path().join("home");
    let run = bilbo(
        dir.path(),
        &[("BILBO_HOME", home.to_str().unwrap())],
        &["restore", "release", "a1b2c3"],
    );
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", home.display())
    );
    assert!(!home.exists());
}

#[test]
fn scope_is_a_verb() {
    let dir = TempDir::new("cli-scope");
    let home = dir.path().join("home");
    let env = [("BILBO_HOME", home.to_str().unwrap())];
    let run = bilbo(dir.path(), &env, &["scope"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "(unassigned)\t0 notes\tembedder any\n");
    assert_eq!(
        run.stderr,
        "bilbo: no scopes declared; add scope.<name>.* keys to $HOME/.config/bilbo/config\n"
    );
    let run = bilbo(dir.path(), &env, &["scope", "set", "work", "x.md"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: scope 'work' is not declared"));
    assert!(!home.exists());
}

#[test]
fn device_is_a_verb() {
    let dir = TempDir::new("cli-device");
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    let env = [
        ("BILBO_HOME", home.to_str().unwrap()),
        ("XDG_STATE_HOME", state.to_str().unwrap()),
    ];
    let run = bilbo(dir.path(), &env, &["device"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "device\tnone\nowner\tnone\n");
    let run = bilbo(dir.path(), &env, &["device", "now"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: unexpected argument 'now'\n"));
    let run = bilbo(dir.path(), &env, &["device", "revoke", "bywater"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(!state.exists());
    assert!(!home.exists());
}

#[test]
fn sync_is_a_verb() {
    let dir = TempDir::new("cli-sync");
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    let config = dir.path().join("config");
    let env = [
        ("BILBO_HOME", home.to_str().unwrap()),
        ("BILBO_CONFIG", config.to_str().unwrap()),
        ("XDG_STATE_HOME", state.to_str().unwrap()),
    ];
    let run = bilbo(dir.path(), &env, &["sync", "now"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: unexpected argument 'now'\n"));
    let run = bilbo(dir.path(), &env, &["sync"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: no store at {}\n", home.display())
    );
    std::fs::create_dir_all(home.join("notes")).unwrap();
    std::fs::write(&config, "").unwrap();
    let run = bilbo(dir.path(), &env, &["sync"]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: no scope syncs; set scope.<name>.sync in {}\n",
            config.display()
        )
    );
    std::fs::write(&config, "scope.personal.sync = file:///srv/bilbo\n").unwrap();
    let run = bilbo(dir.path(), &env, &["sync"]);
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "scope personal file:///srv/bilbo: 0 notes, pushed never, pulled never\nlocal: 0 notes sync nowhere\n"
    );
    assert_eq!(
        run.stderr,
        "bilbo: bilbo watch is not running; nothing syncs\n"
    );
    let run = bilbo(dir.path(), &env, &["sync", "declare", "release", "a\nb"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with("bilbo: the reason must be one line\n")
    );
    assert!(!state.exists());
}

#[test]
fn relay_is_a_verb() {
    let dir = TempDir::new("cli-relay");
    let data = dir.path().join("data");
    let missing = dir.path().join("config");
    let env = [("BILBO_CONFIG", missing.to_str().unwrap())];
    let run = bilbo(dir.path(), &env, &["relay"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.starts_with("bilbo: missing --data <dir>\n"));
    let run = bilbo(
        dir.path(),
        &env,
        &["relay", "--data", data.to_str().unwrap(), "--bogus"],
    );
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("bilbo: unknown option '--bogus'\n"));
    assert!(!data.exists());
    assert!(!missing.exists());
}
