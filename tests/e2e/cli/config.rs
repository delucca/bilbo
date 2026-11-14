use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use super::scratch::Scratch;
use super::{Run, binary};

const KEYS: &str = "paths.root, paths.state, paths.cache, store.seal_at, store.seal_after, \
                    store.compression, store.tear_budget, store.wait, store.poll, follow, read.budget, read.outline_width, \
                    read.matches_listed, search.chunk_tokens, search.chunk_overlap, search.workers, \
                    search.index_memory, search.merge_deleted_percent, search.entry_tries, recall.budget, recall.candidates, recall.cap, recall.record_cap, recall.copy_share, recall.copy_words, recall.question_words, recall.languages, recall.bm25_k1, recall.bm25_b, recall.read_batch, markdown.heading_width, session.unpack_floor, session.unpack_ratio";

#[test]
fn each_key_shows_the_source_that_won_a_variable_over_the_file_over_the_default() {
    let scratch = Scratch::new("precedence");
    let config = scratch.config(
        "[paths]\nstate = \"~/state\"\n\n[store]\nseal_at = \"16 MiB\"\ncompression = 12\n",
    );
    let run = scratch.run_with(&["config"], &[("BILBO_STORE_COMPRESSION", "3")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stderr, "");
    assert_eq!(
        run.stdout,
        format!(
            "paths.root = \"{root}\" (BILBO_HOME)\n\
             paths.state = \"{home}/state\" ({config}:2)\n\
             paths.cache = \"{cache}/bilbo\" (default)\n\
             store.seal_at = \"16 MiB\" ({config}:5)\n\
             store.seal_after = \"60m\" (default)\n\
             store.compression = 3 (BILBO_STORE_COMPRESSION)\n\
             store.tear_budget = 4 (default)\n\
             store.wait = \"10s\" (default)\n\
             store.poll = \"20ms\" (default)\n\
             follow = [{{ path = \"{home}/.claude/projects\", kind = \"session\" }}, \
             {{ path = \"{home}/.codex/sessions\", kind = \"session\" }}] (default)\n\
             read.budget = 7000 (default)\n\
             read.outline_width = 100 (default)\n\
             read.matches_listed = 5 (default)\n\
             search.chunk_tokens = 512 (default)\n\
             search.chunk_overlap = 64 (default)\n\
             search.workers = 0 (default)\n\
             search.index_memory = \"64 MiB\" (default)\n\
             search.merge_deleted_percent = 20 (default)\n\
             search.entry_tries = 3 (default)\n\
             recall.budget = 4000 (default)\n\
             recall.candidates = 400 (default)\n\
             recall.cap = 0 (default)\n\
             recall.record_cap = 2 (default)\n\
             recall.copy_share = 0.5 (default)\n\
             recall.copy_words = 5 (default)\n\
             recall.question_words = [\"what\", \"did\", \"we\", \"should\", \"how\", \"anything\"] (default)\n\
             recall.languages = [\"pt\"] (default)\n\
             recall.bm25_k1 = 1.2 (default)\n\
             recall.bm25_b = 0.75 (default)\n\
             recall.read_batch = 4096 (default)\n\
             markdown.heading_width = 100 (default)\n\
             session.unpack_floor = \"128 MiB\" (default)\n\
             session.unpack_ratio = 256 (default)\n",
            root = scratch.path("store"),
            home = scratch.path("home"),
            cache = scratch.path("cache"),
        )
    );
    let bare = scratch.run(&["config", "store.seal_at"]);
    assert_eq!((bare.code, bare.stdout.as_str()), (0, "16 MiB\n"));
}

#[test]
fn a_bad_value_in_the_file_exits_2_naming_the_file_the_line_and_the_key() {
    let scratch = Scratch::new("bad_value");
    let config = scratch.config("[store]\n\nseal_at = \"0\"\n");
    scratch.write("a.md", "a\n");
    let notes = scratch.path("notes");
    for args in [&["config"][..], &["remember", &notes]] {
        let run = scratch.run(args);
        assert_eq!(run.code, 2, "{args:?}");
        assert_eq!(run.stdout, "", "{args:?}");
        assert_eq!(
            run.stderr,
            format!(
                "bilbo: {config}:3: store.seal_at must be between 1 MiB and 1 GiB, got \"0\"\n"
            ),
            "{args:?}"
        );
    }
    for folder in ["store", "state", "cache"] {
        assert!(!scratch.dir.join(folder).exists(), "{folder}");
    }
}

#[test]
fn a_language_bilbo_does_not_know_is_a_config_error() {
    let scratch = Scratch::new("unknown_language");
    let config = scratch.config("[recall]\nlanguages = [\"es\"]\n");
    let run = scratch.run(&["config"]);
    assert_eq!((run.code, run.stdout.as_str()), (2, ""));
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: {config}:2: recall.languages must be a list of language codes; pt is the one bilbo knows, got \"es\"\n"
        )
    );
}

#[test]
fn a_bad_file_value_exits_2_even_when_its_variable_is_set() {
    let scratch = Scratch::new("bad_value_with_variable");
    let config = scratch.config("[store]\n\nseal_at = \"0\"\n");
    let run = scratch.run_with(&["config"], &[("BILBO_STORE_SEAL_AT", "16 MiB")]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!("bilbo: {config}:3: store.seal_at must be between 1 MiB and 1 GiB, got \"0\"\n")
    );
}

#[test]
fn a_scalar_key_written_as_a_table_exits_2_even_when_bilbo_home_is_set() {
    let scratch = Scratch::new("table_for_scalar");
    let config = scratch.config("[paths.root]\nunknown = 1\n");
    let run = scratch.run(&["config"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: {config}:1: paths.root must be an absolute path or one starting with ~/, \
             got a table\n"
        )
    );
}

#[test]
fn a_tilde_value_with_no_home_exits_2_naming_its_file_line_and_key() {
    let scratch = Scratch::new("tilde_without_home");
    let config = scratch.config("[paths]\nroot = \"~/store\"\n");
    let run = scratch.run_without(&["config"], "HOME");
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!("bilbo: {config}:2: paths.root needs HOME, which is not set\n")
    );
}

#[test]
fn paths_in_the_file_move_the_store_and_the_cache_when_bilbo_home_is_unset() {
    let scratch = Scratch::new("paths_from_file");
    let config = scratch.config(&format!(
        "[paths]\nroot = \"{}\"\ncache = \"{}\"\n",
        scratch.path("file-store"),
        scratch.path("file-cache")
    ));
    let run = scratch.run_without_home(&["config"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        format!(
            "paths.root = \"{}\" ({config}:2)",
            scratch.path("file-store")
        )
    );
    assert_eq!(
        lines[2],
        format!(
            "paths.cache = \"{}\" ({config}:3)",
            scratch.path("file-cache")
        )
    );
    scratch.write("a.md", "a\n");
    let kept = scratch.run_without_home(&["remember", &scratch.path("notes")]);
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    assert!(scratch.dir.join("file-cache/catalog.sqlite").exists());
    scratch.backdate(61);
    let sealing = scratch.run_without_home(&["remember", &scratch.path("notes")]);
    assert_eq!(sealing.code, 0, "{}", sealing.stderr);
    let sealed = fs::read_dir(scratch.dir.join("file-store"))
        .expect("the store the file names")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("entry")
                .path()
                .extension()
                .is_some_and(|extension| extension == "seg")
        })
        .count();
    assert_eq!(sealed, 1);
    assert!(!scratch.dir.join("store").exists() && !scratch.dir.join("cache").exists());
}

#[test]
fn an_unknown_key_in_the_file_exits_2_and_lists_the_keys() {
    let scratch = Scratch::new("unknown_key");
    let config = scratch.config("[store]\nsealat = \"8 MiB\"\n");
    let run = scratch.run(&["config"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        format!("bilbo: {config}:2: unknown key store.sealat; the keys are {KEYS}\n")
    );
}

#[test]
fn a_config_file_bilbo_config_names_must_exist() {
    let scratch = Scratch::new("missing_config");
    fs::remove_file(scratch.dir.join("config/config.toml")).expect("remove the config file");
    let run = scratch.run(&["config"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: BILBO_CONFIG names a file that does not exist: {}\n",
            scratch.path("config/config.toml")
        )
    );
}

#[test]
fn an_unknown_key_named_on_the_command_line_is_a_usage_error() {
    let scratch = Scratch::new("unknown_argument");
    let run = scratch.run(&["config", "store.sealat"]);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!(
            "bilbo: unknown key 'store.sealat'; the keys are {KEYS}\n\
             bilbo: usage: bilbo config [<key>]\n\
             bilbo: see 'bilbo config --help'\n"
        )
    );
}

#[test]
fn help_and_the_version_need_no_valid_config_file() {
    let scratch = Scratch::new("help_broken");
    scratch.config("[store]\nseal_at = \"0\"\n");
    for args in [&["--help"][..], &["config", "--help"], &["--version"]] {
        let run = scratch.run(args);
        assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{args:?}");
    }
}

#[test]
fn remember_works_with_every_folder_and_store_key_set_in_the_file() {
    let scratch = Scratch::new("keys_in_file");
    scratch.config(&format!(
        "[paths]\nstate = \"{}\"\n\n[store]\nseal_at = \"1 MiB\"\nseal_after = \"2m\"\ncompression = 3\ntear_budget = 2\n",
        scratch.path("elsewhere")
    ));
    scratch.write("a.md", "a\nb\n");
    let run = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some("new 1, appended 0, new generation 0, unchanged 0, refused 0")
    );
    assert!(scratch.dir.join("elsewhere/open.seg").exists());
    assert!(!scratch.dir.join("state").exists());
    assert_eq!(scratch.segments(), 0);
    let at = SystemTime::now() - Duration::from_secs(3 * 60);
    let millis = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_millis();
    fs::write(
        scratch.dir.join("elsewhere/open.since"),
        format!("{millis}\n"),
    )
    .expect("backdate");
    assert_eq!(scratch.run(&["remember", &scratch.path("notes")]).code, 0);
    assert_eq!(scratch.segments(), 1, "3 minutes is past the file's 2m");
}

fn stored_bytes(stdout: &str) -> u64 {
    let totals = stdout.lines().nth(1).expect("a totals line");
    totals
        .rsplit("stored bytes ")
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no stored bytes in {totals:?}"))
}

fn compressible_note() -> String {
    let words = [
        "store", "note", "frame", "segment", "seal", "append", "catalog", "digest", "folder",
        "level", "budget", "reader", "writer", "record", "header", "payload",
    ];
    let mut state = 1_u32;
    (0..600)
        .map(|n| {
            let line: Vec<&str> = (0..9)
                .map(|_| {
                    state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    words[(state >> 16) as usize % words.len()]
                })
                .collect();
            format!("line {n}: {}\n", line.join(" "))
        })
        .collect()
}

fn remember_stored_bytes(test: &str, config: &str) -> u64 {
    let scratch = Scratch::new(test);
    scratch.config(config);
    scratch.write("a.md", &compressible_note());
    let run = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    stored_bytes(&run.stdout)
}

#[test]
fn the_store_options_remember_uses_are_the_resolved_ones() {
    let fast = remember_stored_bytes("level_1", "[store]\ncompression = 1\n");
    let small = remember_stored_bytes("level_19", "[store]\ncompression = 19\n");
    assert!(
        fast > small,
        "level 1 stored {fast}, level 19 stored {small}"
    );
}

const ALPHABET: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

fn remember_noisy_notes(test: &str, config: &str) -> Scratch {
    let scratch = Scratch::new(test);
    scratch.config(config);
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    for file in 0..7 {
        let text: String = (0..256 * 1024)
            .map(|n| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                match n % 64 {
                    63 => '\n',
                    _ => char::from(ALPHABET[(state % 62) as usize]),
                }
            })
            .collect();
        scratch.write(&format!("{file}.md"), &text);
    }
    let run = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    scratch
}

#[test]
fn seal_at_from_the_file_decides_when_the_store_seals_a_segment() {
    let small = remember_noisy_notes("seal_1_mib", "[store]\nseal_at = \"1 MiB\"\n");
    assert_eq!(
        small.segments(),
        1,
        "1 MiB seals during the run; the young rest stays open"
    );
    let default = remember_noisy_notes("seal_default", "");
    assert_eq!(default.segments(), 0);
    for scratch in [&small, &default] {
        let open = fs::metadata(scratch.dir.join("state/bilbo/open.seg")).expect("open.seg");
        assert!(open.len() > 12);
    }
}

#[test]
fn seal_after_from_the_file_decides_when_an_old_open_seg_is_sealed() {
    for (config, sealed) in [("[store]\nseal_after = \"5m\"\n", 1), ("", 0)] {
        let scratch = Scratch::new("seal_after");
        scratch.config(config);
        scratch.write("a.md", "a\n");
        assert_eq!(scratch.run(&["remember", &scratch.path("notes")]).code, 0);
        assert_eq!(scratch.segments(), 0, "{config:?}");
        scratch.backdate(10);
        assert_eq!(scratch.run(&["remember", &scratch.path("notes")]).code, 0);
        assert_eq!(scratch.segments(), sealed, "10 minutes against {config:?}");
    }
}

/// Runs bilbo with the scratch environment and fails the test, killing the child, if it has not
/// exited within `limit`.
fn run_within(
    scratch: &Scratch,
    args: &[&str],
    extra: (&'static str, &str),
    limit: Duration,
) -> Run {
    let mut env = scratch.env();
    env.push((extra.0, extra.1.to_owned()));
    let mut child = Command::new(binary())
        .env_clear()
        .envs(env.iter().map(|(name, value)| (*name, value.as_str())))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bilbo starts");
    let started = Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("bilbo {args:?} did not exit within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().expect("output");
    Run {
        code: output.status.code().expect("an exit code"),
        stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
    }
}

#[test]
fn a_config_path_that_is_not_a_regular_file_never_blocks_a_run() {
    let scratch = Scratch::new("config_fifo");
    let fifo = scratch.path("config/fifo");
    let made = Command::new("/usr/bin/mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(made.success(), "mkfifo {fifo}");
    let limit = Duration::from_secs(20);
    for args in [&["--help"][..], &["--version"], &["help"]] {
        let run = run_within(&scratch, args, ("BILBO_CONFIG", &fifo), limit);
        assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{args:?}");
    }
    let expected = format!("bilbo: cannot read {fifo}: not a regular file\n");
    for args in [&["config"][..], &["remember", &scratch.path("notes")]] {
        let run = run_within(&scratch, args, ("BILBO_CONFIG", &fifo), limit);
        assert_eq!(run.code, 2, "{args:?}");
        assert_eq!(run.stdout, "", "{args:?}");
        assert_eq!(run.stderr, expected, "{args:?}");
    }
}

#[test]
fn a_config_path_that_is_a_folder_is_not_a_regular_file_either() {
    let scratch = Scratch::new("config_folder");
    let folder = scratch.path("config");
    let run = scratch.run_with(&["config"], &[("BILBO_CONFIG", &folder)]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        format!("bilbo: cannot read {folder}: not a regular file\n")
    );
}

#[test]
fn a_search_key_out_of_range_is_a_config_error() {
    let scratch = Scratch::new("search_out_of_range");
    for (variable, value, allowed) in [
        ("BILBO_SEARCH_WORKERS", "65", "a whole number from 0 to 64"),
        (
            "BILBO_SEARCH_INDEX_MEMORY",
            "15 MiB",
            "between 16 MiB and 2 GiB",
        ),
        (
            "BILBO_SEARCH_MERGE_DELETED_PERCENT",
            "0",
            "a whole number from 1 to 100",
        ),
        (
            "BILBO_SEARCH_ENTRY_TRIES",
            "10",
            "a whole number from 1 to 9",
        ),
    ] {
        let run = scratch.run_with(&["config"], &[(variable, value)]);
        assert_eq!(run.code, 2, "{variable}");
        assert_eq!(run.stdout, "", "{variable}");
        assert_eq!(
            run.stderr,
            format!("bilbo: {variable} must be {allowed}, got \"{value}\"\n")
        );
    }
}

#[test]
fn a_config_value_with_a_bidirectional_control_prints_as_given_off_a_terminal() {
    let scratch = Scratch::new("bidi_value");
    let folder = scratch.dir.join("store\u{200f}");
    fs::create_dir(&folder).expect("a folder");
    let path = folder.to_str().expect("UTF-8");
    let run = scratch.run_with(&["config", "paths.root"], &[("BILBO_HOME", path)]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, format!("{path}\n"));
}
