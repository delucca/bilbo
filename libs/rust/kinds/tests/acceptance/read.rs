use std::cell::Cell;
use std::path::Path;
use std::time::{Duration, SystemTime};

use bilbo_kinds::read::outline::Line;
use bilbo_kinds::read::page::{Block, Cut};
use bilbo_kinds::read::refs::{Ref, Target};
use bilbo_kinds::read::{self, About, Body, Budget, Error, Output, Request, Shown};
use bilbo_kinds::reader::{Passage, Piece, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Head, Meta, RecordRange,
    StoreError,
};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{CaptureLabel, Label, StoredLabels};

use crate::fakes::id;

const SESSION: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
const OTHER: &str = "5c3a9e1f-ffff-4c2e-9a41-7f2d8e6b1c03";

/// A line is a record: `#` lines render nothing, `user:` lines are prompts, `call <name>` names
/// the tool a later `result` line prints, and `@<seconds> ` before a line is its time.
struct Lines;

impl Reader for Lines {
    fn kind(&self) -> &'static str {
        "lines"
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        let mut tool = "unknown".to_owned();
        let mut passages = Vec::new();
        for (record, number) in records.iter().zip(1u64..) {
            let line = String::from_utf8_lossy(record);
            let mut line = line.trim_end_matches('\n');
            let mut at = None;
            if let Some((seconds, rest)) = line.strip_prefix('@').and_then(|l| l.split_once(' ')) {
                at = seconds
                    .parse()
                    .ok()
                    .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s));
                line = rest;
            }
            if line.starts_with('#') {
                continue;
            }
            let text = match line.split_once(' ') {
                Some(("call", name)) => {
                    name.clone_into(&mut tool);
                    format!("tool_use {name}:")
                }
                Some(("result", body)) => format!("tool_result {tool}:\n{body}"),
                _ => line.to_owned(),
            };
            passages.push(Passage {
                range: RecordRange {
                    first: number,
                    count: 1,
                },
                pieces: vec![Piece {
                    start: 0,
                    label: text.split(':').next().unwrap_or_default().to_owned(),
                }],
                ends: vec![text.len()],
                outline: text.starts_with("user:").then(|| text.clone()),
                text,
                at,
            });
        }
        passages
    }

    fn appends_only(&self) -> bool {
        true
    }
}

/// Sixteen tokens a character, so the least budget the key accepts pages these short records.
struct Wide;

const WIDE: usize = 16;

impl Tokens for Wide {
    fn ends(&self, text: &str) -> Vec<usize> {
        text.char_indices()
            .flat_map(|(at, c)| std::iter::repeat_n(at + c.len_utf8(), WIDE))
            .collect()
    }

    fn count(&self, text: &str) -> usize {
        text.chars().count() * WIDE
    }

    fn prefix(&self, text: &str, budget: usize) -> usize {
        text.char_indices()
            .nth(budget / WIDE)
            .map_or(text.len(), |(at, _)| at)
    }
}

struct Kept {
    meta: Meta,
    generations: Vec<Vec<&'static str>>,
}

#[derive(Default)]
struct Shelf {
    kept: Vec<Kept>,
    reads: Cell<usize>,
}

impl Shelf {
    fn with(self, n: u8, kind: &str, label: Option<&str>, records: &[&'static str]) -> Shelf {
        self.with_id(id(n), kind, label, records)
    }

    fn with_id(
        mut self,
        id: Id,
        kind: &str,
        label: Option<&str>,
        records: &[&'static str],
    ) -> Shelf {
        self.kept.push(Kept {
            meta: Meta {
                id,
                kind: kind.to_owned(),
                labels: StoredLabels {
                    capture: label.and_then(|path| CaptureLabel::file(Path::new(path))),
                    id: None,
                },
                captured: SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
            },
            generations: vec![records.to_vec()],
        });
        self
    }

    fn find(&self, id: Id) -> Option<&Kept> {
        self.kept.iter().find(|kept| kept.meta.id == id)
    }
}

impl Entries for Shelf {
    fn capture(&mut self, _input: &Capture<'_>) -> Result<Captured, CaptureError> {
        panic!("read must not call capture")
    }

    fn set_meta(&mut self, _id: Id, _meta: &Meta) -> Result<u64, StoreError> {
        panic!("read must not call set_meta")
    }

    fn read(
        &self,
        id: Id,
        generation: Generation,
        records: RecordRange,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        self.reads.set(self.reads.get() + 1);
        let kept = self.find(id).expect("read asks for an entry it found");
        let Some(all) = (generation.0 as usize)
            .checked_sub(1)
            .and_then(|at| kept.generations.get(at))
        else {
            return Err(StoreError::Missing(format!(
                "entry {id} has no generation {}",
                generation.0
            )));
        };
        assert_eq!(
            (records.first, records.count),
            (1, all.len() as u64),
            "read fetches the whole generation"
        );
        Ok(all
            .iter()
            .map(|line| format!("{line}\n").into_bytes())
            .collect())
    }

    fn meta(&self, id: Id) -> Result<Option<Meta>, StoreError> {
        Ok(self.find(id).map(|kept| kept.meta.clone()))
    }

    fn changes(&self, _since: &Cursor) -> Result<(Vec<Change>, Cursor), StoreError> {
        panic!("read must not call changes")
    }

    fn by_capture(&self, _label: &CaptureLabel) -> Result<Option<Head>, StoreError> {
        panic!("read must not call by_capture")
    }

    fn extent(
        &self,
        id: Id,
        generation: Option<Generation>,
    ) -> Result<Option<(Generation, u64)>, StoreError> {
        let Some(kept) = self.find(id) else {
            return Ok(None);
        };
        let newest = kept.generations.len() as u32;
        // As the store does: an entry kept with its meta and no generation yet is at 0.
        if generation.is_none() && newest == 0 {
            return Ok(Some((Generation(0), 0)));
        }
        let asked = generation.map_or(newest, |generation| generation.0);
        Ok(kept
            .generations
            .get((asked as usize).wrapping_sub(1))
            .map(|records| (Generation(asked), records.len() as u64)))
    }

    fn captures_with(&self, part: &str) -> Result<Vec<(Id, CaptureLabel)>, StoreError> {
        Ok(self
            .kept
            .iter()
            .filter_map(|kept| Some((kept.meta.id, kept.meta.labels.capture.clone()?)))
            .filter(|(_, label)| label.as_str().to_lowercase().contains(part))
            .collect())
    }

    fn by_shorts(&self, shorts: &[Short]) -> Result<Vec<Id>, StoreError> {
        let mut found: Vec<Id> = self
            .kept
            .iter()
            .map(|kept| kept.meta.id)
            .filter(|id| shorts.iter().any(|short| short.ends(*id)))
            .collect();
        found.sort();
        Ok(found)
    }
}

const RECORDS: [&str; 9] = [
    "# a snapshot",
    "@60 user: plant the beans",
    "assistant: on it",
    "call Bash",
    "# thinking",
    "result two rows",
    "user: now the peas",
    "# an event",
    "assistant: done",
];

fn shelf() -> Shelf {
    let label = format!("/h/.claude/projects/p/{SESSION}.jsonl");
    Shelf::default()
        .with(1, "lines", Some(&label), &RECORDS)
        .with(2, "lines", None, &["user: a note"])
        .with(3, "pdf", Some("/h/a.pdf"), &["x"])
        .with(4, "lines", Some("/h/empty.lines"), &[])
}

fn readers() -> Vec<Box<dyn Reader>> {
    vec![Box::new(Lines)]
}

fn run_with(shelf: &Shelf, reference: &str, outline: bool, budget: usize) -> Result<Output, Error> {
    run_tuned(shelf, reference, outline, budget, (100, 5))
}

fn run_tuned(
    shelf: &Shelf,
    reference: &str,
    outline: bool,
    budget: usize,
    (outline_width, matches_listed): (usize, usize),
) -> Result<Output, Error> {
    let request = Request {
        reference: Ref::parse(reference).expect("a ref"),
        outline,
        budget: Budget::try_from(budget).expect("a budget the key accepts"),
        outline_width,
        matches_listed,
    };
    let fetched = read::fetch(&request, shelf)?;
    let shown = Shown {
        tokens: &Wide,
        beside: &|_| String::new(),
    };
    read::render(&fetched, &request, &readers(), &shown)
}

fn run(shelf: &Shelf, reference: &str) -> Result<Output, Error> {
    run_with(shelf, reference, false, 160_000)
}

fn range(first: u64, last: u64) -> RecordRange {
    RecordRange {
        first,
        count: last - first + 1,
    }
}

fn block(record: u64, text: &str) -> Block {
    Block {
        records: range(record, record),
        text: text.to_owned(),
        cut: None,
    }
}

fn blocks(output: &Output) -> &[Block] {
    match &output.body {
        Body::Page(blocks) => blocks,
        Body::Outline { .. } => panic!("a page was asked"),
    }
}

fn lines(output: &Output) -> (&Option<About>, Vec<(u64, &str)>) {
    match &output.body {
        Body::Outline { about, lines } => (
            about,
            lines
                .iter()
                .map(|Line { records, text }| (records.first, text.as_str()))
                .collect(),
        ),
        Body::Page(_) => panic!("an outline was asked"),
    }
}

fn first() -> String {
    id(1).to_string()
}

#[test]
fn an_entry_with_no_locator_reads_as_its_metadata_and_its_outline() {
    let shelf = shelf();
    let output = run(&shelf, &first()).expect("an outline");
    let (about, listed) = lines(&output);
    assert_eq!(
        about.as_ref(),
        Some(&About {
            kind: "lines".to_owned(),
            labels: vec![Label::Capture(
                CaptureLabel::file(Path::new(&format!("/h/.claude/projects/p/{SESSION}.jsonl")))
                    .expect("a label")
            )],
            more_labels: 0,
            captured: SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
            started: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(60)),
            current: Generation(1),
        })
    );
    assert_eq!(
        listed,
        [(2, "user: plant the beans"), (7, "user: now the peas")]
    );
    assert_eq!(
        (output.id, output.total, output.asked),
        (id(1), 9, range(1, 9))
    );
    assert_eq!(
        (output.generation, output.older, output.more),
        (Generation(1), false, None)
    );
}

#[test]
fn a_range_reads_as_each_record_that_renders_with_its_number() {
    let shelf = shelf();
    let output = run(&shelf, &format!("{}#r2-6", first())).expect("a page");
    assert_eq!(
        blocks(&output),
        [
            block(2, "user: plant the beans"),
            block(3, "assistant: on it"),
            block(4, "tool_use Bash:"),
            block(6, "tool_result Bash:\ntwo rows"),
        ]
    );
    assert_eq!((output.asked, output.more), (range(2, 6), None));
    let one = run(&shelf, &format!("bilbo:{}#r3", first())).expect("a page");
    assert_eq!(blocks(&one), [block(3, "assistant: on it")]);
}

#[test]
fn a_range_sees_what_the_records_before_it_set() {
    let shelf = shelf();
    let output = run(&shelf, &format!("{}#r6", first())).expect("a page");
    assert_eq!(blocks(&output), [block(6, "tool_result Bash:\ntwo rows")]);
    assert_eq!(shelf.reads.get(), 1, "one read of the whole generation");
}

#[test]
fn a_range_that_holds_no_rendered_record_prints_nothing_and_succeeds() {
    let shelf = shelf();
    for locator in ["#r1", "#r5", "#r8"] {
        let output = run(&shelf, &format!("{}{locator}", first())).expect("an empty page");
        assert!(blocks(&output).is_empty(), "{locator}");
        assert_eq!(output.more, None, "{locator}");
    }
}

#[test]
fn a_page_ends_at_the_budget_and_following_more_prints_each_record_once() {
    let shelf = shelf();
    for budget in [256, 320, 416, 752, 960, 160_000] {
        let mut printed = Vec::new();
        let mut reference = format!("{}#r1-9", first());
        let mut pages = 0;
        loop {
            let output = run_with(&shelf, &reference, false, budget).expect("a page");
            printed.extend(blocks(&output).iter().map(|block| block.records.first));
            pages += 1;
            assert!(pages <= 9, "budget {budget}: the chain does not end");
            match output.more {
                Some(more) => reference = format!("{}{}", first(), output.locator(more)),
                None => break,
            }
        }
        assert_eq!(printed, [2, 3, 4, 6, 7, 9], "budget {budget}");
    }
    let page = run_with(&shelf, &format!("{}#r1-9", first()), false, 752).expect("a page");
    assert_eq!(blocks(&page).len(), 2);
    assert_eq!(page.more, Some(range(4, 9)));
}

#[test]
fn a_record_over_the_budget_is_cut_and_its_suggested_budget_prints_it_whole() {
    let shelf = shelf();
    let reference = format!("{}#r6-9", first());
    let page = run_with(&shelf, &reference, false, 320).expect("a page");
    assert_eq!(
        blocks(&page),
        [Block {
            cut: Some(Cut {
                characters: 10,
                budget: Budget::try_from(1000).expect("a budget"),
            }),
            ..block(6, "tool_result Bash")
        }]
    );
    assert_eq!(page.more, Some(range(7, 9)));
    let whole = run_with(&shelf, &format!("{}#r6", first()), false, 1000).expect("a page");
    assert_eq!(blocks(&whole), [block(6, "tool_result Bash:\ntwo rows")]);
}

#[test]
fn an_outline_is_paged_too_and_a_range_can_be_outlined() {
    let shelf = shelf();
    let page = run_with(&shelf, &first(), false, 480).expect("an outline");
    let (about, listed) = lines(&page);
    assert!(about.is_some());
    assert_eq!(listed, [(2, "user: plant the beans")]);
    assert_eq!(page.more, Some(range(7, 9)));
    let rest = run_with(&shelf, &format!("{}#r7-9", first()), true, 480).expect("an outline");
    let (about, listed) = lines(&rest);
    assert!(about.is_none(), "a range has no metadata");
    assert_eq!(listed, [(7, "user: now the peas")]);
    assert_eq!(rest.more, None);
    let whole = run_with(&shelf, &first(), true, 160_000).expect("an outline");
    assert!(
        lines(&whole).0.is_some(),
        "--outline with no range is the bare read"
    );
}

#[test]
fn an_older_generation_is_read_by_its_number_and_marks_every_locator() {
    let mut shelf = shelf();
    shelf.kept[0]
        .generations
        .push(vec!["user: rewritten", "assistant: again"]);
    let current = run(&shelf, &first()).expect("an outline");
    assert_eq!(
        (current.generation, current.older, current.total),
        (Generation(2), false, 2)
    );
    assert_eq!(lines(&current).1, [(1, "user: rewritten")]);

    let older = run(&shelf, &format!("{}#g1", first())).expect("an outline");
    assert_eq!(
        (older.generation, older.older, older.total),
        (Generation(1), true, 9)
    );
    let (about, listed) = lines(&older);
    assert_eq!(
        about.as_ref().map(|about| about.current),
        Some(Generation(2))
    );
    assert_eq!(listed.len(), 2);
    assert_eq!(older.locator(range(2, 6)).to_string(), "#g1:r2-6");

    let page = run_with(&shelf, &format!("{}#g1:r2-9", first()), false, 848).expect("a page");
    assert_eq!(blocks(&page).len(), 2);
    assert_eq!(
        page.more.map(|more| page.locator(more).to_string()),
        Some("#g1:r4-9".to_owned())
    );

    let named = run(&shelf, &format!("{}#g2:r1", first())).expect("a page");
    assert_eq!(
        named.locator(range(1, 1)).to_string(),
        "#r1",
        "the current one needs no mark"
    );
    assert_eq!(blocks(&named), [block(1, "user: rewritten")]);
}

#[test]
fn the_outline_lists_the_capture_label_then_the_id_label() {
    let mut shelf = shelf();
    let capture = CaptureLabel::file(Path::new("/h/sub.jsonl")).expect("a label");
    shelf.kept[1].meta.labels = StoredLabels {
        capture: Some(capture.clone()),
        id: Some(id(1)),
    };
    let output = run(&shelf, &id(2).to_string()).expect("an outline");
    assert_eq!(
        lines(&output).0.as_ref().map(|about| about.labels.clone()),
        Some(vec![Label::Capture(capture), Label::Id(id(1))])
    );
    let main = run(&shelf, &first()).expect("an outline");
    assert_eq!(
        lines(&main).0.as_ref().map(|about| about.labels.len()),
        Some(1)
    );
}

#[test]
fn an_entry_with_no_stored_label_lists_none() {
    let shelf = shelf();
    let note = run(&shelf, &id(2).to_string()).expect("an outline");
    let (about, listed) = lines(&note);
    assert_eq!(
        about
            .as_ref()
            .map(|about| (about.labels.clone(), about.started)),
        Some((Vec::new(), None))
    );
    assert_eq!(listed, [(1, "user: a note")]);
    let empty = run(&shelf, &id(4).to_string()).expect("an outline");
    assert_eq!((empty.total, empty.asked.count, empty.more), (0, 0, None));
    assert!(lines(&empty).1.is_empty());
}

#[test]
fn an_entry_kept_with_its_meta_and_no_generation_reads_as_an_empty_entry() {
    let mut shelf = shelf().with(5, "lines", Some("/h/torn.lines"), &[]);
    shelf.kept[4].generations.clear();
    let empty = run(&shelf, &id(5).to_string()).expect("an outline");
    assert_eq!((empty.total, empty.asked.count, empty.more), (0, 0, None));
    assert!(lines(&empty).1.is_empty());
    assert!(matches!(
        run(&shelf, &format!("{}#r1", id(5))),
        Err(Error::OutOfRange { total: 0, .. })
    ));
    match run(&shelf, &format!("{}#g1", id(5))) {
        Err(Error::NoGeneration { generation, .. }) => assert_eq!(generation, Generation(1)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_caller_says_what_it_prints_around_an_outline_and_the_lines_get_the_rest() {
    let shelf = shelf();
    let listed = |beside: &'static str| {
        let request = Request {
            reference: Ref::parse(&first()).expect("a ref"),
            outline: true,
            budget: Budget::try_from(800).expect("a budget"),
            outline_width: 100,
            matches_listed: 5,
        };
        let fetched = read::fetch(&request, &shelf).expect("the entry");
        let shown = Shown {
            tokens: &Wide,
            beside: &|frame: &Output| {
                assert!(
                    matches!(&frame.body, Body::Outline { about: Some(_), lines } if lines.is_empty())
                );
                assert_eq!((frame.total, frame.more), (9, None));
                beside.to_owned()
            },
        };
        let output = read::render(&fetched, &request, &readers(), &shown).expect("an outline");
        lines(&output).1.len()
    };
    // The two lines cost 386 and 338 tokens: both fit 800, and neither does beside 5 characters.
    assert_eq!(listed(""), 2);
    assert_eq!(listed("12345"), 1);
    assert_eq!(listed("1234"), 2);
}

#[test]
fn a_session_id_names_the_entry_whose_capture_label_ends_in_it() {
    let shelf = shelf();
    for reference in [
        SESSION.to_owned(),
        SESSION.to_uppercase(),
        SESSION[..8].to_owned(),
        SESSION[..13].to_owned(),
    ] {
        let output = run(&shelf, &format!("{reference}#r2")).expect("a page");
        assert_eq!(output.id, id(1), "{reference}");
        assert_eq!(blocks(&output), [block(2, "user: plant the beans")]);
    }
    let rollout = Shelf::default().with(
        7,
        "lines",
        Some(&format!(
            "/h/.codex/sessions/2026/11/06/rollout-2026-11-06T09-10-00-{SESSION}.jsonl"
        )),
        &["user: hi"],
    );
    assert_eq!(run(&rollout, SESSION).expect("an outline").id, id(7));
}

#[test]
fn a_session_id_that_names_no_entry_or_several_is_refused() {
    let label = |uuid: &str| format!("/h/.claude/projects/p/{uuid}.jsonl");
    let shelf = shelf()
        .with(5, "lines", Some(&label(OTHER)), &["user: other"])
        .with(
            6,
            "lines",
            Some(&format!("/h/{SESSION}/subagents/agent-a1.jsonl")),
            &["user: sub"],
        )
        .with(
            7,
            "lines",
            Some(&format!("/h/copy/{SESSION}.jsonl")),
            &["user: copy"],
        );
    match run(&shelf, &SESSION[..8]) {
        Err(Error::Ambiguous { given, entries, .. }) => {
            assert_eq!(given, &SESSION[..8]);
            let ids: Vec<Id> = entries.iter().map(|(id, _)| *id).collect();
            assert_eq!(
                ids,
                [id(1), id(5), id(7)],
                "in id order, the subagent's file left out"
            );
        }
        other => panic!("{other:?}"),
    }
    match run(&shelf, SESSION) {
        Err(Error::Ambiguous { entries, .. }) => assert_eq!(entries.len(), 2),
        other => panic!("{other:?}"),
    }
    assert_eq!(run(&shelf, &OTHER[..13]).expect("one entry").id, id(5));
    for unknown in ["0e1d2c3b", "0e1d2c3b-4a59-4687-b6a5-a4b3c2d1e0f9"] {
        match run(&shelf, unknown) {
            Err(Error::NoEntry(Target::Session(session))) => assert_eq!(session, unknown),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(shelf.reads.get(), 1, "a refused ref reads no record");
}

const SHORT: &str = "2T6W9YB0DE";
const WHOLE: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
const ALIKE: &str = "01JC5Q3V7XKM4P8S2T6W9YB0DE";

fn kept_by(ids: &[&str]) -> Shelf {
    ids.iter().fold(Shelf::default(), |shelf, text| {
        let id = Id::parse(text).expect("an id");
        shelf.with_id(
            id,
            "lines",
            Some(&format!("/h/{text}.lines")),
            &["user: hi"],
        )
    })
}

#[test]
fn an_entry_is_read_by_its_short_id_as_by_its_id() {
    let shelf = kept_by(&[WHOLE, "01JC5Q3V7XKM4P8R2T6W9YB0DF"]);
    let whole = run(&shelf, WHOLE).expect("an outline");
    for reference in [SHORT.to_owned(), format!("bilbo:{SHORT}")] {
        let short = run(&shelf, &reference).expect("an outline");
        assert_eq!(short.id, Id::parse(WHOLE).expect("id"), "{reference}");
        assert_eq!(short.total, whole.total, "{reference}");
        assert_eq!(lines(&short), lines(&whole), "{reference}");
    }
    let page = run(&shelf, &format!("{SHORT}#r1")).expect("a page");
    assert_eq!(blocks(&page), [block(1, "user: hi")]);
}

#[test]
fn a_short_id_two_entries_end_with_is_refused_and_both_are_listed() {
    let shelf = kept_by(&[ALIKE, WHOLE, "01JC5Q3V7XKM4P8R2T6W9YB0DF"]);
    match run(&shelf, SHORT) {
        Err(Error::Ambiguous { given, entries, .. }) => {
            assert_eq!(given, SHORT);
            let ids: Vec<String> = entries.iter().map(|(id, _)| id.to_string()).collect();
            assert_eq!(ids, [WHOLE, ALIKE]);
            assert!(entries.iter().all(|(_, label)| label.is_some()));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(shelf.reads.get(), 0, "a refused ref reads no record");
}

#[test]
fn a_short_id_no_entry_ends_with_is_no_entry() {
    let shelf = shelf().with(5, "lines", Some(&format!("/h/{SESSION}.jsonl")), &["x"]);
    match run(&shelf, SHORT) {
        Err(error @ Error::NoEntry(Target::Short(_))) => {
            assert_eq!(error.to_string(), "no entry's id ends with 2T6W9YB0DE");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_outline_width_cuts_a_line_where_the_request_says() {
    let shelf = Shelf::default().with(
        1,
        "lines",
        None,
        &["user: abcdefghijklmnopqrstuvwxyz0123456789"],
    );
    let text = |width| {
        let output = run_tuned(&shelf, &first(), true, 160_000, (width, 5)).expect("an outline");
        match output.body {
            Body::Outline { lines, .. } => lines[0].text.clone(),
            Body::Page(_) => panic!("an outline"),
        }
    };
    assert_eq!(text(20), "user: abcdefghijklm…");
    assert_eq!(text(23), "user: abcdefghijklmnop…");
    assert_eq!(text(100), "user: abcdefghijklmnopqrstuvwxyz0123456789");
}

#[test]
fn an_ambiguous_session_lists_as_many_entries_as_the_request_says() {
    let mut shelf = Shelf::default();
    for number in 1..=4 {
        shelf = shelf.with(
            number,
            "lines",
            Some(&format!("/h/{number}/{SESSION}.jsonl")),
            &["user: copy"],
        );
    }
    for (listed, shown, more) in [
        (1, 1, Some(3)),
        (2, 2, Some(2)),
        (4, 4, None),
        (100, 4, None),
    ] {
        let Err(error) = run_tuned(&shelf, &SESSION[..8], false, 160_000, (100, listed)) else {
            panic!("a session that names four entries");
        };
        let text = error.to_string();
        assert_eq!(text.matches("(file:").count(), shown, "{listed}");
        match more {
            Some(more) => assert!(text.contains(&format!(" and {more} more;")), "{text}"),
            None => assert!(!text.contains(" more"), "{text}"),
        }
    }
}

#[test]
fn an_unknown_id_a_missing_generation_and_a_range_past_the_end_are_refused() {
    let shelf = shelf();
    let unknown = id(9);
    match run(&shelf, &unknown.to_string()) {
        Err(Error::NoEntry(Target::Entry(found))) => assert_eq!(found, unknown),
        other => panic!("{other:?}"),
    }
    match run(&shelf, &format!("{}#g2", first())) {
        Err(Error::NoGeneration {
            id: found,
            generation,
        }) => {
            assert_eq!((found, generation), (id(1), Generation(2)));
        }
        other => panic!("{other:?}"),
    }
    for (locator, asked) in [
        ("#r10", range(10, 10)),
        ("#r5-10", range(5, 10)),
        ("#r400-500", range(400, 500)),
    ] {
        match run(&shelf, &format!("{}{locator}", first())) {
            Err(Error::OutOfRange {
                id: found,
                total,
                asked: refused,
            }) => {
                assert_eq!((found, total, refused), (id(1), 9, asked), "{locator}");
            }
            other => panic!("{locator}: {other:?}"),
        }
    }
    assert!(matches!(
        run(&shelf, &format!("{}#r1", id(4))),
        Err(Error::OutOfRange { total: 0, .. })
    ));
    assert!(run(&shelf, &format!("{}#r9", first())).is_ok());
    assert_eq!(shelf.reads.get(), 1, "a refused ref reads no record");
}

#[test]
fn an_entry_of_a_kind_with_no_reader_is_refused() {
    let shelf = shelf();
    match run(&shelf, &id(3).to_string()) {
        Err(Error::UnreadKind { id: found, kind }) => {
            assert_eq!((found, kind.as_str()), (id(3), "pdf"));
        }
        other => panic!("{other:?}"),
    }
}

/// `#label <text>` records name a label each and render nothing; the rest read as `Lines` does.
struct Folders;

impl Reader for Folders {
    fn kind(&self) -> &'static str {
        "folders"
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        Lines.passages(records)
    }

    fn appends_only(&self) -> bool {
        true
    }

    fn entry_labels(&self, records: &[&[u8]]) -> Vec<String> {
        records
            .iter()
            .filter_map(|record| std::str::from_utf8(record).ok())
            .filter_map(|line| line.trim_end_matches('\n').strip_prefix("#label "))
            .map(str::to_owned)
            .collect()
    }
}

struct Bytes;

impl Tokens for Bytes {
    fn ends(&self, text: &str) -> Vec<usize> {
        text.char_indices()
            .flat_map(|(at, c)| std::iter::repeat_n(at + c.len_utf8(), c.len_utf8()))
            .collect()
    }
}

fn row(frame: &Output) -> String {
    let Body::Outline {
        about: Some(about), ..
    } = &frame.body
    else {
        return String::new();
    };
    let mut text = String::new();
    for label in &about.labels {
        match label {
            Label::Capture(label) => text += &format!("label: {label} (capture)\n"),
            Label::Id(id) => text += &format!("label: bilbo:{id}\n"),
            Label::Text(label) => text += &format!("label: {label}\n"),
        }
    }
    if about.more_labels > 0 {
        text += &format!("labels: {} more\n", about.more_labels);
    }
    text
}

fn folders(label: Option<&str>, found: &[String]) -> Shelf {
    let mut records: Vec<&'static str> = found
        .iter()
        .map(|text| &*Box::leak(format!("#label {text}").into_boxed_str()))
        .collect();
    records.push("user: a note");
    Shelf::default().with(1, "folders", label, &records)
}

fn outlined(shelf: &Shelf, budget: usize) -> Output {
    let request = Request {
        reference: Ref::parse(&first()).expect("a ref"),
        outline: false,
        budget: Budget::try_from(budget).expect("a budget"),
        outline_width: 100,
        matches_listed: 5,
    };
    let fetched = read::fetch(&request, shelf).expect("the entry");
    let shown = Shown {
        tokens: &Bytes,
        beside: &row,
    };
    let readers: Vec<Box<dyn Reader>> = vec![Box::new(Folders)];
    read::render(&fetched, &request, &readers, &shown).expect("an outline")
}

fn texts(output: &Output) -> Vec<&str> {
    lines(output)
        .0
        .as_ref()
        .expect("an outline with its metadata")
        .labels
        .iter()
        .filter_map(|label| match label {
            Label::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn more(output: &Output) -> u64 {
    lines(output)
        .0
        .as_ref()
        .map_or(0, |about| about.more_labels)
}

#[test]
fn the_outline_lists_what_the_reader_finds_in_byte_order() {
    let capture = CaptureLabel::file(Path::new("/h/x.folders")).expect("a label");
    let found = [
        "/h/src/shed".to_owned(),
        "/h/notes".to_owned(),
        capture.as_str().to_owned(),
        "/h/Zed".to_owned(),
    ];
    let mut shelf = folders(Some("/h/x.folders"), &found);
    shelf.kept[0].meta.labels.id = Some(id(9));
    let output = outlined(&shelf, 7000);
    let about = lines(&output).0.clone().expect("an outline");
    assert_eq!(
        about.labels,
        [
            Label::Capture(capture),
            Label::Id(id(9)),
            Label::Text("/h/Zed".to_owned()),
            Label::Text("/h/notes".to_owned()),
            Label::Text("/h/src/shed".to_owned()),
        ]
    );
    assert_eq!(about.more_labels, 0);
}

#[test]
fn a_reader_with_no_label_leaves_the_stored_ones() {
    let mut shelf = shelf();
    shelf.kept[0].meta.labels.id = Some(id(9));
    let output = run(&shelf, &first()).expect("an outline");
    let about = lines(&output).0.clone().expect("an outline");
    assert_eq!(about.labels.len(), 2);
    assert!(matches!(about.labels[0], Label::Capture(_)));
    assert_eq!(about.labels[1], Label::Id(id(9)));
    assert_eq!(about.more_labels, 0);
}

#[test]
fn the_texts_listed_cost_at_most_half_the_budget_and_the_rest_are_counted() {
    let found: Vec<String> = (0..20)
        .map(|n| format!("/t/{n:02}{}", "x".repeat(35)))
        .collect();
    let shelf = folders(None, &found);
    let output = outlined(&shelf, 1000);
    assert_eq!(
        texts(&output),
        found[..10].iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(more(&output), 10);
    assert!(Bytes.count(&row(&output)) <= 500);
    assert!(!lines(&output).1.is_empty());
    let output = outlined(&shelf, 2000);
    assert_eq!(texts(&output).len(), 20);
    assert_eq!(more(&output), 0);
}

#[test]
fn the_stored_labels_are_listed_whatever_the_budget() {
    let path = format!("/h/{}", "d/".repeat(100));
    let found: Vec<String> = (0..5).map(|n| format!("/t/{n}")).collect();
    let mut shelf = folders(Some(&path), &found);
    shelf.kept[0].meta.labels.id = Some(id(9));
    let output = outlined(&shelf, 256);
    let about = lines(&output).0.clone().expect("an outline");
    assert_eq!(about.labels.len(), 2);
    assert!(matches!(about.labels[0], Label::Capture(_)));
    assert_eq!(about.labels[1], Label::Id(id(9)));
    assert_eq!(about.more_labels, 5);
}

#[test]
fn the_most_labels_a_reader_can_give_are_bounded_at_the_smallest_budget() {
    let found: Vec<String> = (0..256)
        .map(|n| format!("/{n:04}{}", "d".repeat(4091)))
        .collect();
    let shelf = folders(None, &found);
    let output = outlined(&shelf, 256);
    assert!(texts(&output).is_empty());
    assert_eq!(more(&output), 256);
    assert_eq!(lines(&output).1, [(257, "user: a note")]);
}
