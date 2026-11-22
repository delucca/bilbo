use std::fs;
use std::time::Duration;

use bilbo_llm::child::Bounds;
use bilbo_llm::secret::{self, Secret, Source};

use crate::Scratch;

const KEY: &str = "derive.test.key_command";
const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(5);

fn nothing(_: &str) -> Option<String> {
    None
}

fn program(scratch: &Scratch, body: &str) -> Vec<String> {
    let script = scratch.script("key.sh", body);
    vec![
        "/bin/sh".to_owned(),
        script.to_str().expect("a text path").to_owned(),
    ]
}

fn run(scratch: &Scratch, body: &str, timeout: Duration, limit: usize) -> Result<Secret, String> {
    let command = program(scratch, body);
    secret::read(
        Source::choose("UNUSED_VARIABLE", &command),
        KEY,
        &nothing,
        &scratch.path().join("derive"),
        Bounds {
            limit,
            timeout,
            poll: POLL,
        },
    )
}

#[test]
fn the_key_is_the_variables_value_when_no_program_is_set() {
    let reader = |name: &str| (name == "TEST_KEY").then(|| "sk-from-variable".to_owned());
    let secret = secret::read(
        Source::choose("TEST_KEY", &[]),
        "derive.test.key_env",
        &reader,
        &std::env::temp_dir(),
        Bounds {
            limit: 1024,
            timeout: WAIT,
            poll: POLL,
        },
    )
    .unwrap();
    assert_eq!(secret.expose(), "sk-from-variable");
}

#[test]
fn an_unset_or_empty_variable_is_refused_by_its_name() {
    let reader = |name: &str| (name == "EMPTY_KEY").then(String::new);
    for name in ["UNSET_KEY", "EMPTY_KEY"] {
        let error = secret::read(
            Source::choose(name, &[]),
            "derive.test.key_env",
            &reader,
            &std::env::temp_dir(),
            Bounds {
                limit: 1024,
                timeout: WAIT,
                poll: POLL,
            },
        )
        .unwrap_err();
        assert_eq!(error, format!("{name} is not set (derive.test.key_env)"));
    }
}

#[test]
fn the_program_wins_and_its_first_line_is_the_key() {
    let scratch = Scratch::new("secret");
    let command = program(&scratch, "echo sk-from-program\necho second line");
    let reader = |_: &str| Some("sk-from-variable".to_owned());
    let secret = secret::read(
        Source::choose("TEST_KEY", &command),
        KEY,
        &reader,
        &scratch.path().join("derive"),
        Bounds {
            limit: 1024,
            timeout: WAIT,
            poll: POLL,
        },
    )
    .unwrap();
    assert_eq!(secret.expose(), "sk-from-program");
}

#[test]
fn a_program_that_fails_is_reported_by_its_exit_code_alone() {
    let scratch = Scratch::new("secret");
    let error = run(
        &scratch,
        "echo canary-out\necho canary-err >&2\nexit 3",
        WAIT,
        1024,
    )
    .unwrap_err();
    assert_eq!(error, format!("{KEY} failed: exit 3"));
    assert!(!error.contains("canary"));
}

#[test]
fn a_program_that_prints_nothing_or_does_not_end_is_refused() {
    let scratch = Scratch::new("secret");
    assert_eq!(
        run(&scratch, "exit 0", WAIT, 1024).unwrap_err(),
        format!("{KEY} printed no key")
    );
    assert_eq!(
        run(&scratch, "sleep 30", Duration::from_secs(1), 1024).unwrap_err(),
        format!("{KEY} did not end in \"1s\"")
    );
}

#[test]
fn a_program_that_prints_too_much_or_is_stopped_by_a_signal_is_refused() {
    let scratch = Scratch::new("secret");
    assert_eq!(
        run(&scratch, "printf '%01025d' 0", WAIT, 1024).unwrap_err(),
        format!("{KEY} printed more than llm.response allows")
    );
    assert_eq!(
        run(&scratch, "kill -9 $$", WAIT, 1024).unwrap_err(),
        format!("{KEY} ended with no exit code")
    );
}

#[test]
fn the_keys_program_gets_bilbos_environment_and_runs_in_the_derive_folder() {
    let scratch = Scratch::new("secret");
    let folder = scratch.path().join("state").join("derive");
    assert!(!folder.exists());
    let command = program(&scratch, "echo \"$PWD|$PATH\"");
    let secret = secret::read(
        Source::choose("UNUSED", &command),
        KEY,
        &nothing,
        &folder,
        Bounds {
            limit: 4096,
            timeout: WAIT,
            poll: POLL,
        },
    )
    .unwrap();
    let path = std::env::vars_os()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.to_string_lossy().into_owned())
        .unwrap();
    assert_eq!(secret.expose(), format!("{}|{path}", folder.display()));
    assert!(fs::metadata(&folder).unwrap().is_dir());
}

#[test]
fn a_key_is_kept_without_the_white_space_around_it() {
    let padded = |name: &str| (name == "PADDED").then(|| "  sk-test-PLUM-7391\t".to_owned());
    let secret = secret::read(
        Source::choose("PADDED", &[]),
        "derive.test.key_env",
        &padded,
        &std::env::temp_dir(),
        Bounds {
            limit: 1024,
            timeout: WAIT,
            poll: POLL,
        },
    )
    .unwrap();
    assert_eq!(secret.expose(), "sk-test-PLUM-7391");
    assert!(secret.hides(&["x sk-test-PLUM-7391 y"]));
    let blank = |name: &str| (name == "BLANK").then(|| "  ".to_owned());
    let error = secret::read(
        Source::choose("BLANK", &[]),
        "derive.test.key_env",
        &blank,
        &std::env::temp_dir(),
        Bounds {
            limit: 1024,
            timeout: WAIT,
            poll: POLL,
        },
    )
    .unwrap_err();
    assert_eq!(error, "BLANK is not set (derive.test.key_env)");
    let scratch = Scratch::new("secret-white");
    let key = run(&scratch, "echo '  sk-test-PLUM-7391  '", WAIT, 1024).unwrap();
    assert_eq!(key.expose(), "sk-test-PLUM-7391");
    assert_eq!(
        run(&scratch, "printf '\\t\\n'", WAIT, 1024).unwrap_err(),
        format!("{KEY} printed no key")
    );
}
