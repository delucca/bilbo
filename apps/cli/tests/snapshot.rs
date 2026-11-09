use std::ffi::OsString;
use std::path::PathBuf;

use bilbo_cli::console::env::Env;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings::{self, Settings};
use bilbo_cli::console::view::View;
use bilbo_cli::remember::view;
use bilbo_cli::run;
use bilbo_kinds::remember::{Item, Outcome, Output};
use bilbo_store::id::Id;

fn args(list: &[&str]) -> Vec<OsString> {
    std::iter::once("bilbo")
        .chain(list.iter().copied())
        .map(OsString::from)
        .collect()
}

fn unread() -> Result<Settings, Failure> {
    Err(Failure::Config(
        "these tests never read a setting".to_owned(),
    ))
}

/// Every default, under a fixed HOME and with no config file.
fn defaults() -> Settings {
    settings::load(&|name| (name == "HOME").then(|| "/home/u".into()), &|_| {
        Err(std::io::ErrorKind::NotFound.into())
    })
    .expect("the defaults")
}

fn stdout(list: &[&str], view: &View) -> String {
    let outcome = run(&args(list), &mut Env::quiet(), unread()).expect("a result");
    assert_eq!(outcome.exit_code(), 0, "{list:?}");
    assert!(outcome.stderr_lines(view).is_empty(), "{list:?}");
    outcome.stdout(view)
}

fn stderr(list: &[&str], view: &View) -> String {
    let failure = run(&args(list), &mut Env::quiet(), unread()).expect_err("a failure");
    assert_eq!(failure.exit_code(), 2, "{list:?}");
    let mut text = failure.stderr_lines(view).join("\n");
    text.push('\n');
    text
}

fn unpainted() -> View {
    View {
        paint: false,
        ..View::HUMAN
    }
}

#[test]
fn the_overview_is_the_same_from_every_help_form() {
    let overview = include_str!("snapshots/overview.txt");
    for form in [
        &["--help"][..],
        &["-h"],
        &["help"],
        &["help", "help"],
        &["help", "--help"],
        &["--help", "--frobnicate"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), overview, "{form:?}");
    }
}

#[test]
fn the_overview_on_a_terminal() {
    assert_eq!(
        stdout(&["--help"], &View::HUMAN),
        include_str!("snapshots/overview.human.ansi")
    );
    assert_eq!(
        stdout(&["--help"], &unpainted()),
        include_str!("snapshots/overview.txt")
    );
}

#[test]
fn the_remember_page_on_a_terminal() {
    assert_eq!(
        stdout(&["remember", "--help"], &View::HUMAN),
        include_str!("snapshots/remember-help.human.ansi")
    );
    assert_eq!(
        stdout(&["remember", "--help"], &unpainted()),
        include_str!("snapshots/remember-help.txt")
    );
}

#[test]
fn the_config_page_on_a_terminal() {
    assert_eq!(
        stdout(&["config", "--help"], &View::HUMAN),
        include_str!("snapshots/config-help.human.ansi")
    );
    assert_eq!(
        stdout(&["config", "--help"], &unpainted()),
        include_str!("snapshots/config-help.txt")
    );
}

#[test]
fn the_version_is_bilbo_and_the_build_version() {
    let expected = format!("bilbo {}\n", env!("CARGO_PKG_VERSION"));
    for form in [&["--version"][..], &["-V"]] {
        assert_eq!(stdout(form, &View::PLAIN), expected);
        assert_eq!(stdout(form, &View::HUMAN), expected);
    }
}

#[test]
fn a_top_level_usage_error_in_the_plain_view_is_the_reason_and_the_page() {
    assert_eq!(
        stderr(&[], &View::PLAIN),
        include_str!("snapshots/usage-missing-verb.txt")
    );
    assert_eq!(
        stderr(&["--frobnicate"], &View::PLAIN),
        include_str!("snapshots/usage-unknown-option.txt")
    );
    for form in [&["frob"][..], &["help", "frob"], &["frob", "--help"]] {
        assert_eq!(
            stderr(form, &View::PLAIN),
            include_str!("snapshots/usage-unknown-verb.txt"),
            "{form:?}"
        );
    }
}

#[test]
fn a_usage_error_on_a_terminal_is_a_marked_message() {
    assert_eq!(
        stderr(&["frob"], &View::HUMAN),
        include_str!("snapshots/usage-unknown-verb.human.ansi")
    );
    assert_eq!(
        stderr(&["frob"], &unpainted()),
        "■  unknown verb 'frob'; the verbs are remember, config\n   see bilbo --help\n"
    );
}

fn remembered() -> Output {
    let item = |name: &str, outcome| Item {
        path: PathBuf::from(name),
        outcome,
    };
    let id = |byte: u8| Id::from_bytes([byte; 16]);
    Output {
        target: PathBuf::from("/notes"),
        kind: Some("markdown".to_owned()),
        claimed: 8,
        items: vec![
            item("/notes/bad.md", Outcome::Refused("not UTF-8".to_owned())),
            item("/notes/a.md", Outcome::New(id(1))),
            item("/notes/b.md", Outcome::New(id(2))),
            item("/notes/c.md", Outcome::Appended(id(3))),
            item("/notes/d.md", Outcome::NewGeneration(id(4))),
            item("/notes/e.md", Outcome::Unchanged(id(5))),
            item("/notes/f.md", Outcome::Unchanged(id(6))),
            item("/notes/g.md", Outcome::Unchanged(id(7))),
        ],
        records: 1234,
        raw_bytes: 56789,
        stored_bytes: 12345,
    }
}

#[test]
fn the_remember_output_in_the_plain_view() {
    assert_eq!(
        view::plain(&remembered()),
        include_str!("snapshots/remember-output.txt")
    );
}

#[test]
fn the_remember_output_on_a_terminal() {
    assert_eq!(
        view::human(&remembered(), &View::HUMAN),
        include_str!("snapshots/remember-output.human.ansi")
    );
}

#[test]
fn the_remember_page_is_the_same_from_every_help_form() {
    let page = include_str!("snapshots/remember-help.txt");
    for form in [
        &["remember", "--help"][..],
        &["remember", "-h"],
        &["help", "remember"],
        &["help", "remember", "--help"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), page, "{form:?}");
    }
}

fn configured(list: &[&str], view: &View) -> String {
    let outcome = run(&args(list), &mut Env::quiet(), Ok(defaults())).expect("a result");
    assert_eq!(outcome.exit_code(), 0, "{list:?}");
    outcome.stdout(view)
}

#[test]
fn the_config_listing_in_the_plain_view() {
    assert_eq!(
        configured(&["config"], &View::PLAIN),
        include_str!("snapshots/config-output.txt")
    );
}

#[test]
fn the_config_listing_on_a_terminal() {
    assert_eq!(
        configured(&["config"], &View::HUMAN),
        include_str!("snapshots/config-output.human.ansi")
    );
}

#[test]
fn the_config_page_is_the_same_from_every_help_form() {
    let page = include_str!("snapshots/config-help.txt");
    for form in [
        &["config", "--help"][..],
        &["config", "-h"],
        &["help", "config"],
        &["help", "config", "--help"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), page, "{form:?}");
    }
}
