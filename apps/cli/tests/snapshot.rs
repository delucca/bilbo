use std::ffi::OsString;
use std::path::PathBuf;

use bilbo_cli::console::env::Env;
use bilbo_cli::console::outcome::Failure;
use bilbo_cli::console::settings::{self, Settings};
use bilbo_cli::console::view::View;
use bilbo_cli::remember::view;
use bilbo_cli::{kinds, readers, run};
use bilbo_entries::files::Memory;
use bilbo_entries::read::refs::Ref;
use bilbo_entries::read::{self, Fetched};
use bilbo_entries::remember::{self, Item, Outcome, Output, Request, Tagging};
use bilbo_store::entries::{Entries, Generation, Meta, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::labels::{By, CaptureLabel, Standing, StoredLabels, Tag, Tags};
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
        &bilbo_cli::provider_keys(),
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
        "■  unknown verb 'frob'; the verbs are remember, recall, read, tag, reindex, derive, config\n   see bilbo --help\n"
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
        tagged: 0,
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

#[test]
fn the_tag_page_is_the_same_from_every_help_form_and_on_a_terminal() {
    let page = include_str!("snapshots/tag-help.txt");
    for form in [
        &["tag", "--help"][..],
        &["tag", "-h"],
        &["help", "tag"],
        &["help", "tag", "--help"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), page, "{form:?}");
    }
    assert_eq!(
        stdout(&["tag", "--help"], &View::HUMAN),
        include_str!("snapshots/tag-help.human.ansi")
    );
    assert_eq!(stdout(&["tag", "--help"], &unpainted()), page);
}

#[test]
fn the_tag_listings_in_the_plain_view() {
    use bilbo_entries::tag::Carrier;
    use bilbo_store::labels::{By, Tagged};
    let tagged = |name: &str, kind: &str, entries: u64| Tagged {
        tag: Tag::new(name).expect("a tag"),
        kind: kind.to_owned(),
        entries,
    };
    let list = bilbo_cli::tag::view::tags(&[
        tagged("carpentry-docs", "markdown", 40),
        tagged("garden", "markdown", 4),
        tagged("garden", "session", 209),
        tagged("rust", "markdown", 117),
    ]);
    assert_eq!(list, include_str!("snapshots/tag-list.txt"));
    let carrier = |id: &str, kind: &str, capture: Option<&str>, by| Carrier {
        id: Id::parse(id).expect("an id"),
        kind: kind.to_owned(),
        capture: capture.and_then(|text| CaptureLabel::new(text.to_owned())),
        by,
    };
    let entries = bilbo_cli::tag::view::entries(&[
        carrier(
            "01JC5Q3V7XKM4P8R2T6W9YB0DB",
            "session",
            Some(
                "file:/home/me/.claude/projects/-home-me-src-garden/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
            ),
            By::Person,
        ),
        carrier(
            "01JC5Q3V7XKM4P8R2T6W9YB0DC",
            "markdown",
            Some("file:/home/me/notes/garden-log.md"),
            By::Model,
        ),
        carrier("01JC5Q3V7XKM4P8R2T6W9YB0DD", "markdown", None, By::Person),
    ]);
    assert_eq!(entries, include_str!("snapshots/tag-entries.txt"));
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
        "../../../libs/entries/rust/fixtures/session/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
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
    let kinds = kinds(&defaults()).expect("the kinds");
    let request = Request {
        target: PathBuf::from(path),
        kind: None,
        tags: Tagging::default(),
    };
    let plan = remember::plan(&request, &kinds, &files).expect("plan");
    let mut store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open");
    remember::keep(&plan, &files, &mut store).expect("keep");
    let head = store
        .by_capture(&CaptureLabel::new(format!("file:{path}")).expect("capture label"))
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
    "../../../libs/entries/rust/fixtures/session/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"
);
const CODEX_SESSION: &[u8] = include_bytes!(
    "../../../libs/entries/rust/fixtures/session/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
);
const NOTE: &[u8] = include_bytes!("../../../libs/entries/rust/fixtures/markdown/sections.md");

fn read_output(
    kind: &str,
    label: &str,
    file: &[u8],
    locator: &str,
    budget: usize,
) -> (String, String) {
    read_tagged(kind, label, file, locator, budget, Tags::default())
}

fn read_tagged(
    kind: &str,
    label: &str,
    file: &[u8],
    locator: &str,
    budget: usize,
    tags: Tags,
) -> (String, String) {
    let fetched = Fetched {
        meta: Meta {
            id: Id::parse(READ_ID).expect("id"),
            kind: kind.to_owned(),
            labels: StoredLabels {
                capture: CaptureLabel::new(format!("file:{label}")),
                id: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").filter(|_| kind == "session"),
                tags,
            },
            captured: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_193_920),
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
    let label =
        "/home/me/.claude/projects/-home-me-src-garden/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl";
    read_output("session", label, CLAUDE_SESSION, locator, budget)
}

fn codex(locator: &str) -> (String, String) {
    let label = "/home/me/.codex/sessions/2026/11/07/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl";
    read_output("session", label, CODEX_SESSION, locator, 7000)
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
fn an_outline_with_tags_as_read_prints_it() {
    let mut tags = Tags::default();
    tags.set(
        Tag::new("garden").expect("a tag"),
        Some(Standing::Carried(By::Person)),
    );
    tags.set(
        Tag::new("rust").expect("a tag"),
        Some(Standing::Carried(By::Model)),
    );
    tags.set(Tag::new("shed").expect("a tag"), Some(Standing::Removed));
    let label =
        "/home/me/.claude/projects/-home-me-src-garden/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl";
    let (plain, _) = read_tagged("session", label, CLAUDE_SESSION, "", 7000, tags);
    assert_eq!(plain, include_str!("snapshots/read-tagged-outline.txt"));
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

fn recalled() -> bilbo_search::recall::Output {
    use bilbo_search::index::Extent;
    use bilbo_search::lookup::Found;
    use bilbo_search::recall::hit::Hit;
    let hit = |entry: &str,
               kind: &str,
               first,
               count,
               part,
               preface: Option<&str>,
               text: &str,
               cut,
               shared_short| Hit {
        found: Found {
            entry: Id::parse(entry).expect("an id"),
            kind: kind.to_owned(),
            extent: Extent {
                generation: Generation(1),
                records: RecordRange { first, count },
                part,
                date: 1_794_069_552_000,
            },
            digest: 0,
        },
        preface: preface.map(str::to_owned),
        text: text.to_owned(),
        cut,
        apart: false,
        headed: false,
        shared_short,
    };
    bilbo_search::recall::Output {
        hits: vec![
            hit(
                "01JC5Q3V7XKM4P8R2T6W9YB0DE",
                "session",
                3,
                5,
                None,
                Some("Session started 2026-11-07 08:01 UTC:"),
                "user: Thin the carrot rows.\nassistant: Done.\n=== 2T6W9YB0DF#r1\t1 line",
                false,
                false,
            ),
            hit(
                "01JC5Q3V7XKM4P8R2T6W9YB0DG",
                "markdown",
                8,
                1,
                Some((1, 3)),
                None,
                "## Beds\n\nThe north bed holds the carrots.\n",
                false,
                true,
            ),
            hit(
                "01JC5Q3V7XKM4P8R2T6W9YB0DH",
                "session",
                2,
                1,
                None,
                Some("Session date unknown:"),
                "user: and the last one, cut at the budget",
                true,
                false,
            ),
        ],
        searched: 1412,
        stand: bilbo_search::recall::Stand::Current,
        left_out: 0,
        skipped: bilbo_search::recall::select::Skipped::default(),
        scope: None,
        apart: Vec::new(),
        expanded: Vec::new(),
    }
}

#[test]
fn the_recall_output_in_the_plain_view_and_on_a_terminal() {
    assert_eq!(
        bilbo_cli::recall::view::plain(&recalled()),
        include_str!("snapshots/recall-output.txt")
    );
    assert_eq!(
        bilbo_cli::recall::view::human(&recalled(), &View::HUMAN),
        include_str!("snapshots/recall-output.human.ansi")
    );
}

fn recalled_with_notes() -> bilbo_search::recall::Output {
    use bilbo_search::index::Extent;
    use bilbo_search::lookup::Found;
    use bilbo_search::recall::hit::Hit;
    let mut output = recalled();
    output.hits.truncate(2);
    let note = |entry: &str, first, text: &str, cut| Hit {
        found: Found {
            entry: Id::parse(entry).expect("an id"),
            kind: "note".to_owned(),
            extent: Extent {
                generation: Generation(1),
                records: RecordRange { first, count: 1 },
                part: None,
                date: 1_794_069_552_000,
            },
            digest: 0,
        },
        preface: None,
        text: text.to_owned(),
        cut,
        apart: true,
        headed: false,
        shared_short: false,
    };
    output.apart = vec![
        note(
            "01M6ZGE4KTBYYH07AS2KVMSCAF",
            3,
            "01JC5Q3V7XKM4P8R2T6W9YB0DE#g1:r3-19 2026-11-07\nThe user asked to thin the carrot rows.",
            false,
        ),
        note(
            "01M6ZGE4KTBYYH07AS2KVMSCAG",
            1,
            "01JC5Q3V7XKM4P8R2T6W9YB0DE#g1:r20-31 01JC5Q3V7XKM4P8R2T6W9YB0DE#g1:r40 2026-11-08\nThe beans were sown along the south fence, and the",
            true,
        ),
    ];
    output
}

#[test]
fn the_recall_output_with_notes_in_the_plain_view_and_on_a_terminal() {
    assert_eq!(
        bilbo_cli::recall::view::plain(&recalled_with_notes()),
        include_str!("snapshots/recall-notes.txt")
    );
    assert_eq!(
        bilbo_cli::recall::view::human(&recalled_with_notes(), &View::HUMAN),
        include_str!("snapshots/recall-notes.human.ansi")
    );
}

#[test]
fn a_note_entry_as_read_prints_it() {
    let derived = include_bytes!("../../../libs/entries/rust/fixtures/note/derived.jsonl");
    let (plain, _) = read_output("note", "/derive/parent", derived, "#r1-7", 7000);
    assert_eq!(plain, include_str!("snapshots/read-notes-page.txt"));
}

#[test]
fn the_recall_page_is_the_same_from_every_help_form_and_on_a_terminal() {
    let page = include_str!("snapshots/recall-help.txt");
    for form in [
        &["recall", "--help"][..],
        &["recall", "-h"],
        &["help", "recall"],
        &["help", "recall", "--help"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), page, "{form:?}");
    }
    assert_eq!(
        stdout(&["recall", "--help"], &View::HUMAN),
        include_str!("snapshots/recall-help.human.ansi")
    );
    assert_eq!(stdout(&["recall", "--help"], &unpainted()), page);
}

#[test]
fn recall_says_what_was_searched_and_that_nothing_matched_on_stderr() {
    let nothing = bilbo_search::recall::Output {
        hits: Vec::new(),
        searched: 1412,
        stand: bilbo_search::recall::Stand::Current,
        left_out: 0,
        skipped: bilbo_search::recall::select::Skipped::default(),
        scope: None,
        apart: Vec::new(),
        expanded: Vec::new(),
    };
    let finished = bilbo_search::recall::Finished {
        result: Ok(nothing),
        unlogged: None,
    };
    let outcome = bilbo_cli::recall::outcome(finished, &bilbo_store::segments::Cache::Absent)
        .expect("an outcome");
    assert_eq!(outcome.exit_code(), 1);
    let mut text = outcome.stderr_lines(&View::PLAIN).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/recall-nothing.txt"));
}

#[test]
fn the_typos_line_in_the_plain_view() {
    use bilbo_search::rank::Expanded;
    let expanded = |term: &str, near: &[&str]| Expanded {
        term: term.to_owned(),
        near: near.iter().map(|word| (*word).to_owned()).collect(),
    };
    let mut output = recalled();
    output.expanded = vec![
        expanded("carot", &["carrot"]),
        expanded("setin", &["set", "setton"]),
    ];
    let finished = bilbo_search::recall::Finished {
        result: Ok(output),
        unlogged: None,
    };
    let outcome = bilbo_cli::recall::outcome(finished, &bilbo_store::segments::Cache::Absent)
        .expect("an outcome");
    assert_eq!(outcome.exit_code(), 0);
    let mut text = outcome.stderr_lines(&View::PLAIN).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/recall-typos.txt"));
}

#[test]
fn recall_says_what_it_passed_over_on_stderr() {
    let mut output = recalled();
    output.skipped = bilbo_search::recall::select::Skipped {
        copies: 3,
        parts: 1,
        capped: 0,
    };
    let finished = bilbo_search::recall::Finished {
        result: Ok(output),
        unlogged: None,
    };
    let outcome = bilbo_cli::recall::outcome(finished, &bilbo_store::segments::Cache::Absent)
        .expect("an outcome");
    assert_eq!(outcome.exit_code(), 0);
    let mut text = outcome.stderr_lines(&View::PLAIN).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/recall-passed-over.txt"));
}

#[test]
fn an_unknown_kind_of_recall_is_a_usage_error_naming_the_kinds() {
    let failure = run(
        &args(&["recall", "--kind", "nope", "carrots"]),
        &mut Env::quiet(),
        Ok(defaults()),
    )
    .expect_err("a usage error");
    assert_eq!(failure.exit_code(), 2);
    let mut text = failure.stderr_lines(&View::PLAIN).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/recall-usage-kind.txt"));
}

#[test]
fn recall_says_what_a_scope_named_kept_and_left_out_on_stderr() {
    use bilbo_search::recall::Scoped;
    use bilbo_search::recall::scope::Unnamed;
    let mut output = recalled();
    output.skipped = bilbo_search::recall::select::Skipped {
        copies: 3,
        parts: 1,
        capped: 0,
    };
    output.scope = Some(Scoped {
        kept: 1203,
        unnamed: vec![
            ("shd".to_owned(), Unnamed::Nothing),
            (
                "2T6W9YB0DE".to_owned(),
                Unnamed::Several(vec![
                    Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("an id"),
                    Id::parse("01JC5Q3V7XKM4P8S2T6W9YB0DE").expect("an id"),
                ]),
            ),
        ],
        worded: true,
        best: 400,
        outside: [("markdown".to_owned(), 7), ("session".to_owned(), 24)].into(),
    });
    let finished = bilbo_search::recall::Finished {
        result: Ok(output),
        unlogged: None,
    };
    let outcome = bilbo_cli::recall::outcome(finished, &bilbo_store::segments::Cache::Absent)
        .expect("an outcome");
    assert_eq!(outcome.exit_code(), 0);
    let mut text = outcome.stderr_lines(&View::PLAIN).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/recall-scope.txt"));
}

#[test]
fn a_tag_scheme_with_no_tags_name_is_a_usage_error_with_the_usage_on_two_lines() {
    let failure = run(
        &args(&["recall", "--in", "tag:Rust", "carrots"]),
        &mut Env::quiet(),
        Ok(defaults()),
    )
    .expect_err("a usage error");
    assert_eq!(failure.exit_code(), 2);
    let mut text = failure.stderr_lines(&View::PLAIN).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/recall-usage-tag.txt"));
}

#[test]
fn a_value_with_no_letter_or_digit_is_a_usage_error_with_the_usage_on_two_lines() {
    let failure = run(
        &args(&["recall", "--in", ".", "carrots"]),
        &mut Env::quiet(),
        Ok(defaults()),
    )
    .expect_err("a usage error");
    assert_eq!(failure.exit_code(), 2);
    let mut text = failure.stderr_lines(&View::PLAIN).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/recall-usage-in.txt"));
}

#[test]
fn the_derive_page_is_the_same_from_every_help_form_and_on_a_terminal() {
    let page = include_str!("snapshots/derive-help.txt");
    for form in [
        &["derive", "--help"][..],
        &["derive", "-h"],
        &["help", "derive"],
        &["help", "derive", "--help"],
    ] {
        assert_eq!(stdout(form, &View::PLAIN), page, "{form:?}");
    }
    assert_eq!(
        stdout(&["derive", "--help"], &View::HUMAN),
        include_str!("snapshots/derive-help.human.ansi")
    );
    assert_eq!(stdout(&["derive", "--help"], &unpainted()), page);
}

fn derived() -> bilbo_derive::run::Counts {
    bilbo_derive::run::Counts {
        entries: 3,
        calls: 4,
        notes: 31,
        dropped: 2,
        refused: 0,
        failed: 0,
        passed: 1,
        tokens_in: 91_806,
        tokens_out: 7_340,
    }
}

#[test]
fn the_derive_output_in_the_plain_view_and_on_a_terminal() {
    assert_eq!(
        bilbo_cli::derive::view::plain(&derived(), 48),
        include_str!("snapshots/derive-output.txt")
    );
    assert_eq!(
        bilbo_cli::derive::view::human(&derived(), 48, &View::HUMAN),
        include_str!("snapshots/derive-output.human.ansi")
    );
}

#[test]
fn the_dry_run_in_the_plain_view() {
    use bilbo_cli::derive::view::{Dry, KindRow, NotDue, lines};
    use bilbo_derive::instruction::Origin;
    let dry = Dry {
        off: vec!["derive.enabled is false".to_owned()],
        kinds: vec![
            KindRow {
                kind: "session".to_owned(),
                origin: Origin::Shipped,
                instruction: "9f86d081884c7d65".to_owned(),
            },
            KindRow {
                kind: "markdown".to_owned(),
                origin: Origin::File(PathBuf::from("/home/me/notes/markdown.txt")),
                instruction: "4c1a70b9e2d355f0".to_owned(),
            },
        ],
        named: 412,
        left_out: 14,
        due: 37,
        not_due: NotDue {
            growing: 12,
            covered: 340,
            belongs: 9,
            passed: 2,
            labels: 1,
        },
        calls: 20,
        entries: 14,
        tokens: 380_122,
        limit: 20,
        provider: vec![
            "provider: anthropic, via api, model claude-haiku-5-5, at https://api.anthropic.com/v1/messages"
                .to_owned(),
            "version: 2026-10-21".to_owned(),
            "key: the variable ANTHROPIC_API_KEY, which is set".to_owned(),
        ],
        entry_lines: vec![
            "entry\t01M6ZGE4KTBYYH07AS2KVMSCAF\tcovered\t01M7Q9TDMQ9SBDXT8T7VKDGZVH\t12".to_owned(),
            "entry\t01M6ZGF2D0XH5T0S4B7W1N9RQK\tleft out: derive.not_in\t-\t0".to_owned(),
        ],
    };
    let mut text = lines(&dry).join("\n");
    text.push('\n');
    assert_eq!(text, include_str!("snapshots/derive-dry-run.txt"));
}
