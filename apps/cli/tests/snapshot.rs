use std::ffi::OsString;
use std::path::PathBuf;

use bilbo_cli::console::env::Env;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings::{self, Settings};
use bilbo_cli::console::view::View;
use bilbo_cli::remember::view;
use bilbo_cli::{kinds, readers, run};
use bilbo_kinds::files::Memory;
use bilbo_kinds::read::refs::Ref;
use bilbo_kinds::read::{self, Fetched};
use bilbo_kinds::remember::{self, Item, Outcome, Output, Request};
use bilbo_store::entries::{Entries, Generation, Meta, Origin, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Options, Store};

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
    settings::load(
        &|name| (name == "HOME").then(|| "/home/u".into()),
        &|_| Err(std::io::ErrorKind::NotFound.into()),
        &bilbo_cli::kind_keys(),
    )
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
        "■  unknown verb 'frob'; the verbs are remember, read, reindex, config\n   see bilbo --help\n"
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

#[test]
fn the_output_of_a_bare_run_adds_up_every_follow() {
    let mut sessions = remembered();
    sessions.target = PathBuf::from("/h/.claude/projects");
    sessions.kind = Some("session".to_owned());
    assert_eq!(
        view::plain_totals(&view::Totals::of(&[remembered(), sessions])),
        include_str!("snapshots/remember-follows.txt")
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn scratch(name: &str) -> PathBuf {
    std::env::var_os("TEST_TMPDIR")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("{name}-{}", std::process::id()))
}

#[test]
fn a_codex_session_as_the_store_keeps_it() {
    const ROLLOUT: &[u8] = include_bytes!(
        "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
    );
    let path =
        "/s/2026/11/06/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl";
    let dir = scratch("stored_session");
    let layout = Layout {
        root: dir.join("store"),
        state: dir.join("state"),
        cache: dir.join("cache"),
    };
    let files = Memory::new().file(path, ROLLOUT);
    let kinds = kinds();
    let request = Request {
        target: PathBuf::from(path),
        kind: None,
    };
    let plan = remember::plan(&request, &kinds, &files).expect("plan");
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open");
    remember::keep(&plan, &files, &mut store).expect("keep");
    let head = store
        .by_origin(&Origin::new(format!("file:{path}")).expect("origin"))
        .expect("head")
        .expect("an entry");
    let records = store
        .read(
            head.id,
            head.generation,
            RecordRange {
                first: 1,
                count: head.records,
            },
        )
        .expect("read");
    let stored = String::from_utf8(records.concat()).expect("UTF-8");
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        stored,
        include_str!("snapshots/session-rollout.stored.jsonl")
    );
}

#[test]
fn the_read_page_on_a_terminal() {
    assert_eq!(
        stdout(&["read", "--help"], &View::HUMAN),
        include_str!("snapshots/read-help.human.ansi")
    );
    assert_eq!(
        stdout(&["read", "--help"], &unpainted()),
        include_str!("snapshots/read-help.txt")
    );
}

#[test]
fn the_read_page_is_the_same_from_every_help_form() {
    let page = include_str!("snapshots/read-help.txt");
    for form in [
        &["read", "--help"][..],
        &["read", "-h"],
        &["help", "read"],
        &["help", "read", "--help"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), page, "{form:?}");
    }
}

const READ_ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DG";
const CLAUDE_SESSION: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
);
const CODEX_SESSION: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
);
const NOTE: &[u8] = include_bytes!("../../../libs/rust/kinds/markdown/fixtures/sections.md");

fn read_output(
    kind: &str,
    origin: &str,
    file: &[u8],
    locator: &str,
    budget: usize,
) -> (String, String) {
    let fetched = Fetched {
        meta: Meta {
            id: Id::parse(READ_ID).expect("id"),
            kind: kind.to_owned(),
            origin: Origin::new(format!("file:{origin}")),
            captured: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_193_920),
            parent: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").filter(|_| kind == "session"),
        },
        generation: Generation(1),
        current: Generation(1),
        records: file
            .split_inclusive(|byte| *byte == b'\n')
            .map(<[u8]>::to_vec)
            .collect(),
    };
    let budget = read::Budget::try_from(budget).expect("a budget the key accepts");
    let request = read::Request {
        reference: Ref::parse(&format!("{READ_ID}{locator}")).expect("a ref"),
        outline: false,
        budget,
        outline_width: 100,
        matches_listed: 5,
    };
    let given = (budget.tokens() != 7000).then_some(budget);
    let output = bilbo_cli::read::rendered(
        &fetched,
        &request,
        &readers(&defaults()).expect("the readers"),
        given,
    )
    .expect("an output");
    (
        bilbo_cli::read::view::plain(&output, given),
        bilbo_cli::read::view::shown(&output, given, &View::HUMAN),
    )
}

fn claude(locator: &str, budget: usize) -> (String, String) {
    let origin =
        "/home/me/.claude/projects/-home-me-src-garden/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl";
    read_output("session", origin, CLAUDE_SESSION, locator, budget)
}

fn codex(locator: &str) -> (String, String) {
    let origin = "/home/me/.codex/sessions/2026/11/07/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl";
    read_output("session", origin, CODEX_SESSION, locator, 7000)
}

fn note(locator: &str, budget: usize) -> (String, String) {
    read_output(
        "markdown",
        "/home/me/notes/garden-log.md",
        NOTE,
        locator,
        budget,
    )
}

#[test]
fn a_claude_code_session_as_read_prints_it() {
    let (plain, human) = claude("", 7000);
    assert_eq!(plain, include_str!("snapshots/read-claude-outline.txt"));
    assert_eq!(
        human,
        include_str!("snapshots/read-claude-outline.human.ansi")
    );
    let (plain, human) = claude("#r1-19", 7000);
    assert_eq!(plain, include_str!("snapshots/read-claude-page.txt"));
    assert_eq!(human, include_str!("snapshots/read-claude-page.human.ansi"));
}

#[test]
fn a_page_that_ends_at_the_budget_and_a_record_cut_to_it() {
    let (plain, _) = claude("#r1-23", 256);
    assert_eq!(plain, include_str!("snapshots/read-claude-more.txt"));
    let (plain, human) = claude("#r21-23", 256);
    assert_eq!(plain, include_str!("snapshots/read-claude-cut.txt"));
    assert_eq!(human, include_str!("snapshots/read-claude-cut.human.ansi"));
}

#[test]
fn a_codex_session_as_read_prints_it() {
    let (plain, _) = codex("");
    assert_eq!(plain, include_str!("snapshots/read-codex-outline.txt"));
    let (plain, human) = codex("#r1-25");
    assert_eq!(plain, include_str!("snapshots/read-codex-page.txt"));
    assert_eq!(human, include_str!("snapshots/read-codex-page.human.ansi"));
}

#[test]
fn a_note_as_read_prints_it() {
    let (plain, human) = note("", 7000);
    assert_eq!(plain, include_str!("snapshots/read-note-outline.txt"));
    assert_eq!(
        human,
        include_str!("snapshots/read-note-outline.human.ansi")
    );
    let (plain, human) = note("#r8-27", 300);
    assert_eq!(plain, include_str!("snapshots/read-note-page.txt"));
    assert_eq!(human, include_str!("snapshots/read-note-page.human.ansi"));
}

#[test]
fn the_reindex_page_on_a_terminal() {
    assert_eq!(
        stdout(&["reindex", "--help"], &View::HUMAN),
        include_str!("snapshots/reindex-help.human.ansi")
    );
    assert_eq!(
        stdout(&["reindex", "--help"], &unpainted()),
        include_str!("snapshots/reindex-help.txt")
    );
}

#[test]
fn the_reindex_page_is_the_same_from_every_help_form() {
    let page = include_str!("snapshots/reindex-help.txt");
    for form in [
        &["reindex", "--help"][..],
        &["reindex", "-h"],
        &["help", "reindex"],
        &["help", "reindex", "--help"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), page, "{form:?}");
    }
}

fn reindexed() -> bilbo_search::work::Output {
    bilbo_search::work::Output {
        entries: 5335,
        chunks: 1_042_276,
        unread: [("pdf".to_owned(), 2), ("audio".to_owned(), 1)].into(),
        failed: vec![(
            Id::from_bytes([7; 16]),
            "/s/a.seg is damaged at byte 12: checksum mismatch".to_owned(),
        )],
        index_bytes: 198_424_967,
    }
}

#[test]
fn the_reindex_output_in_the_plain_view() {
    assert_eq!(
        bilbo_cli::reindex::view::plain(&reindexed(), 811_190),
        include_str!("snapshots/reindex-output.txt")
    );
}

#[test]
fn the_reindex_output_on_a_terminal() {
    assert_eq!(
        bilbo_cli::reindex::view::human(&reindexed(), 811_190, &View::HUMAN),
        include_str!("snapshots/reindex-output.human.ansi")
    );
}

#[test]
fn the_reindex_lines_for_what_was_left_out() {
    let mut lines: Vec<String> = bilbo_cli::reindex::view::left_out(&reindexed())
        .iter()
        .flat_map(bilbo_cli::console::message::Message::plain_lines)
        .collect();
    lines.push(String::new());
    assert_eq!(
        lines.join("\n"),
        include_str!("snapshots/reindex-left-out.txt")
    );
}
