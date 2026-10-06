mod common;

use common::{TempDir, bilbo, bilbo_input};

const USAGE: &str = "\
usage: bilbo new <kind> <topic> [--title <text>] [--scope <name>]
       bilbo check
       bilbo recall <query>... [--kind <kind>]... [--limit <n>]
       bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]
       bilbo index
       bilbo setup [--yes | --interactive] [--remove] [<setup option>]...
       bilbo digest
       bilbo library [<corpus>]
       bilbo library show <corpus>/<name>|<id>[#<anchor>] [--depth <n>]
       bilbo library stage <url> | <file> --origin \"<url|doc>: <value>\" [--fetched <YYYY-MM-DD>] [--html]
       bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]
       bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]
       bilbo library read <plan> <slice>... [--part <k>/<n>]
       bilbo cite [--plan <plan>]... [<file> | -]
       bilbo watch
       bilbo history <note> [<version> | --diff <a> [<b>]]
       bilbo restore <note> <version>
       bilbo scope
       bilbo scope set [--force] <name> <file>...
       bilbo device
       bilbo device list
       bilbo device init [--name <name>]
       bilbo device recover [--name <name>]
       bilbo device revoke <device>
       bilbo sync
       bilbo sync declare <note> <reason>
       bilbo pair [--scope <name>]... [--via <url>]
       bilbo pair <code> --via <url> [--name <name>]
       bilbo relay --data <dir> --owner <fingerprint>... [--listen <address:port>] [--max-scopes <n>] [--max-scope-mb <n>] [--max-object-mb <n>]
       bilbo --help
       bilbo --version
new creates <root>/notes/<kind>-<topic>.md and prints its path.
check prints every problem in the store and changes nothing.
recall prints the notes that best match the query, best first, 10 unless --limit says otherwise.
recall --library searches the sources and guides of the library by keyword instead of the notes; --corpus narrows it.
index embeds the passages the vector cache lacks and drops the ones no note holds any more.
digest reads a prompt hook's JSON on stdin and prints the notes that bear on the prompt; it always exits 0.
library lists the corpora, prints a corpus's guide with the facts of each source, or a source's outline; stage and land add a source; plan cuts picks into slices and partitions; read prints slices and logs them.
cite checks every bilbo: citation in a draft, and with --plan prints the coverage of the plans' reads.
watch records a version of each note when it changes, until it is stopped.
history lists the versions of a note, newest first, prints one, or shows what changed between two versions, or between one and the note's file now.
restore writes a past version of a note back as its newest version, keeping what the note held before.
scope lists the scopes this device declares with their note counts; scope set gives notes a scope.
device shows this device, its owner and each scope's manifest; device list prints the owner's devices; device init makes this device's keys, with a recovery phrase to write down, and each syncing scope's manifest; device recover reads that phrase on another device and adds it to the manifests; device revoke removes a device from them; init, for a new phrase, recover and revoke need a terminal.
sync prints each syncing scope's state, its devices, the open conflicts and the dropped text nobody declared, and exits 1 when something needs attention; sync declare records that a note's dropped text was dropped on purpose.
pair shows a one-time code on an enrolled device and waits; pair <code> --via <url> on another device joins it to the scopes paired once the user confirms on the first, which needs a terminal.
relay serves the sync transport to the devices of the --owner fingerprints, over plain HTTP under /v1/ behind a TLS proxy, until it is stopped.
setup creates the store and the config and installs the agent plugin, the index timer, the note watcher and, when asked, the local embedder; in a terminal it asks first.
setup options: --embedder-url <url>, --embedder-model <name>, --embedder-token-env <var>, --embedder-token-file <path>, --embedder-query-prefix <text>, --embedder-local, --embedder-port <port>, --llama-server <path>, --no-plugin, --claude <path>, --codex <path>, --plugin-source <folder|owner/repo#ref>, --no-timer, --index-every <minutes>, --no-watch
kinds: plan, spec, design, decision, gotcha, research, review, report, reference
root: $BILBO_HOME, else $XDG_DATA_HOME/bilbo, else $HOME/.local/share/bilbo
config: $BILBO_CONFIG, else $XDG_CONFIG_HOME/bilbo/config, else $HOME/.config/bilbo/config
cache: $XDG_CACHE_HOME/bilbo, else $HOME/.cache/bilbo
state: $XDG_STATE_HOME/bilbo, else $HOME/.local/state/bilbo
";

fn prefixed_usage() -> String {
    USAGE.lines().map(|l| format!("bilbo: {l}\n")).collect()
}

#[test]
fn no_arguments_is_usage_error() {
    let dir = TempDir::new("cli-none");
    let run = bilbo(dir.path(), &[], &[]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!("bilbo: missing verb\n{}", prefixed_usage())
    );
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
    assert!(run.stderr.contains("bilbo: unknown verb 'frobnicate'\n"));
    assert!(
        run.stderr.contains("bilbo new")
            && run.stderr.contains("bilbo check")
            && run.stderr.contains("bilbo recall")
            && run.stderr.contains("bilbo index")
            && run.stderr.contains("bilbo setup")
            && run.stderr.contains("bilbo digest")
            && run.stderr.contains("bilbo library")
            && run.stderr.contains("bilbo cite")
            && run.stderr.contains("bilbo watch")
            && run.stderr.contains("bilbo history")
            && run.stderr.contains("bilbo restore")
            && run.stderr.contains("bilbo scope")
            && run.stderr.contains("bilbo device")
            && run.stderr.contains("bilbo sync")
            && run.stderr.contains("bilbo pair")
            && run.stderr.contains("bilbo relay")
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
    for flag in ["--help", "-h"] {
        let run = bilbo(dir.path(), &[], &[flag]);
        assert_eq!(run.code, 0);
        assert_eq!(run.stdout, USAGE);
        assert!(run.stderr.is_empty());
    }
}

#[test]
fn help_after_a_verb_prints_help() {
    let dir = TempDir::new("cli-help-verb");
    for args in [
        ["check", "--help"],
        ["new", "-h"],
        ["recall", "--help"],
        ["index", "--help"],
        ["setup", "--help"],
    ] {
        let run = bilbo(dir.path(), &[], &args);
        assert_eq!(run.code, 0);
        assert_eq!(run.stdout, USAGE);
        assert!(run.stderr.is_empty());
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
        format!("bilbo: unknown option '--version'\n{}", prefixed_usage())
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
    let run = bilbo(dir.path(), &env, &["device", "revoke", "bagend"]);
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
