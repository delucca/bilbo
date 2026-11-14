use std::collections::BTreeMap;

use bilbo_kinds::reader::Reader;
use bilbo_search::analyze::{Detect, Language, MAX_LABEL_WORDS, Omit, label_words};
use bilbo_search::bm25::Bm25;
use bilbo_search::chunk::Size;
use bilbo_search::lookup::Lookup;
use bilbo_search::position::Manifest;
use bilbo_search::rank::{Candidate, Query};
use bilbo_search::recall::hit::Hit;
use bilbo_search::recall::log::QueryLog;
use bilbo_search::recall::scope::Unnamed;
use bilbo_search::recall::select::{Limits, Skipped};
use bilbo_search::recall::{Access, Error, Finished, Output, Reads, Request, Scoped, Stand, run};
use bilbo_search::reindex;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_search::update::{self, Plan};
use bilbo_search::work::Request as Work;
use bilbo_store::id::Id;

use crate::fakes::{
    Broken, Bytes, Dated, Docs, Failing, Fenced, Joined, Lines, Log, Logged, Never, Says, Scripted,
    at, id,
};

const SIZE: Size = Size {
    tokens: 64,
    overlap: 8,
};

const SHOWN_HEADER: usize = 4 + 26 + 1;

const OFF: Limits = Limits {
    per_entry: 0,
    per_record: 0,
    copy_share: 0.0,
    copy_words: 5,
};

const PASS: Limits = Limits {
    per_entry: 0,
    per_record: 2,
    copy_share: 0.5,
    copy_words: 5,
};

fn manifest() -> Manifest {
    Manifest::new(SIZE, "bytes/1", "lines/1")
}

fn tokenizer() -> Tokenizer {
    Tokenizer {
        tokens: Box::new(Bytes),
        id: "bytes/1",
    }
}

fn readers() -> Vec<Box<dyn Reader>> {
    vec![
        Box::new(Lines("note")),
        Box::new(Dated("session")),
        Box::new(Lines("memo")),
        Box::new(Fenced("fenced")),
        Box::new(Joined("joined")),
    ]
}

fn work_request() -> Work {
    Work {
        chunk: SIZE,
        render: "lines/1".to_owned(),
        workers: 1,
        label_batch: 512,
    }
}

fn index_of(log: &Log) -> Docs {
    let mut index = Docs::default();
    reindex::run(
        &work_request(),
        log,
        log,
        &readers(),
        &tokenizer(),
        &mut index,
    )
    .expect("a run");
    log.reads.borrow_mut().clear();
    index
}

fn request(text: &str) -> Request {
    Request {
        query: Query {
            text: text.to_owned(),
            kinds: Vec::new(),
            entries: None,
        },
        manifest: manifest(),
        chunk: SIZE,
        candidates: 400,
        limits: OFF,
        scope: Vec::new(),
        budget: 100_000,
        at: at(1_700_000_000),
    }
}

fn of_kinds(text: &str, kinds: &[&str]) -> Request {
    let mut request = request(text);
    request.query.kinds = kinds.iter().map(|kind| (*kind).to_owned()).collect();
    request
}

fn shown(hit: &Hit) -> String {
    format!("=== {}\n{}", hit.found.entry, hit.text)
}

fn ask_with(
    log: &Log,
    lookup: &dyn Lookup,
    request: &Request,
    kept: &mut dyn QueryLog,
) -> Finished {
    let bm25 = Bm25 {
        index: lookup,
        k1: 1.2,
        b: 0.75,
        omit: Omit::default(),
        batch: 4096,
    };
    run(
        request,
        &Reads {
            entries: log,
            labels: log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: lookup,
            rankers: &[&bm25],
        },
        &shown,
        kept,
    )
}

fn ask(log: &Log, index: &Docs, request: &Request) -> (Output, Logged) {
    let mut kept = Logged::default();
    let finished = ask_with(log, index, request, &mut kept);
    assert_eq!(finished.unlogged, None);
    (finished.result.expect("a run"), kept)
}

fn fields(kept: &Logged) -> Vec<String> {
    assert_eq!(kept.0.len(), 1, "one line a query: {:?}", kept.0);
    let line = kept.0[0].strip_suffix('\n').expect("a whole line");
    let fields: Vec<String> = line.split('\t').map(str::to_owned).collect();
    assert_eq!(fields.len(), 9, "{line:?}");
    assert_eq!(fields[0], "q2");
    fields
}

fn entries_of(output: &Output) -> Vec<Id> {
    output.hits.iter().map(|hit| hit.found.entry).collect()
}

fn texts(output: &Output) -> Vec<&str> {
    output.hits.iter().map(|hit| hit.text.as_str()).collect()
}

const RELEASE: &str = "release checklist tag the build and publish";
const OTHER: &str = "grocery list bread milk eggs and butter";

fn shelf_of_six() -> Log {
    let mut log = Log::new();
    log.put(1, "note", &[OTHER]);
    log.put(
        2,
        "note",
        &["decided bilbo index format two with a digest field"],
    );
    log.put(3, "note", &["standup moved to thursday morning"]);
    log.put(4, "note", &["the dog needs a walk after lunch today"]);
    log.put(5, "note", &["flight to lisbon leaves at nine"]);
    log.put(6, "note", &["rust borrow checker lifetimes explained"]);
    log
}

fn candidates(index: &Docs, docs: &[u32]) -> Vec<Candidate> {
    Lookup::found(index, docs)
        .expect("rows")
        .into_iter()
        .zip(0..)
        .map(|(found, place)| Candidate {
            found,
            score: 10.0 - place as f32,
        })
        .collect()
}

#[test]
fn last_months_decision_is_found() {
    let log = shelf_of_six();
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &request("bilbo index format"));
    assert_eq!(output.stand, Stand::Current);
    assert_eq!(output.searched, 6);
    assert_eq!(output.left_out, 0);
    assert_eq!(entries_of(&output)[0], id(2));
    assert_eq!(
        output.hits[0].text,
        "decided bilbo index format two with a digest field"
    );
    assert_eq!(output.hits[0].preface, None);
    assert!(!output.hits[0].cut);
}

#[test]
fn a_note_and_a_session_come_back_under_their_kinds() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "session", &[RELEASE]);
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    let mut kinds: Vec<(&str, Id)> = output
        .hits
        .iter()
        .map(|hit| (hit.found.kind.as_str(), hit.found.entry))
        .collect();
    kinds.sort();
    assert_eq!(kinds, [("note", id(1)), ("session", id(2))]);
    assert!(output.hits.iter().all(|hit| hit.text == RELEASE));
}

#[test]
fn a_kind_narrows_the_hits_and_two_kinds_widen_them() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "session", &[RELEASE]);
    log.put(3, "memo", &[RELEASE]);
    log.put(4, "note", &[OTHER]);
    let index = index_of(&log);
    let kinds = |kinds: &[&str]| {
        let (output, _) = ask(&log, &index, &of_kinds("release checklist", kinds));
        let mut found: Vec<String> = output
            .hits
            .iter()
            .map(|hit| hit.found.kind.clone())
            .collect();
        found.sort();
        found
    };
    assert_eq!(kinds(&["note"]), ["note"]);
    assert_eq!(kinds(&["note", "session"]), ["note", "session"]);
    assert_eq!(kinds(&[]), ["memo", "note", "session"]);
    assert!(kinds(&["nothing-of-this-kind"]).is_empty());
}

#[test]
fn a_query_that_matches_nothing_ends_with_no_hit_and_the_count_searched() {
    let log = shelf_of_six();
    let index = index_of(&log);
    for text in ["zzzzzz", "the and of", "what"] {
        let (output, kept) = ask(&log, &index, &request(text));
        assert_eq!(
            output,
            Output {
                hits: Vec::new(),
                searched: 6,
                stand: Stand::Current,
                left_out: 0,
                skipped: Skipped::default(),
                scope: None,
            },
            "{text}"
        );
        assert_eq!(fields(&kept)[2], "nothing");
    }
}

#[test]
fn a_sessions_hit_opens_with_its_kinds_preface_and_a_notes_with_none() {
    let mut log = Log::new();
    log.put(
        1,
        "session",
        &["unrelated opening line", RELEASE, "closing remark here"],
    );
    log.put(2, "note", &[RELEASE]);
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    let preface = |entry: Id| {
        output
            .hits
            .iter()
            .find(|hit| hit.found.entry == entry)
            .map(|hit| hit.preface.clone())
    };
    assert_eq!(preface(id(1)), Some(Some("3 records:".to_owned())));
    assert_eq!(preface(id(2)), Some(None));
}

#[test]
fn a_store_with_no_index_is_not_searched() {
    let mut log = shelf_of_six();
    log.changes_fail = true;
    let scripted = Scripted::new(Vec::new());
    let mut kept = Logged::default();
    let finished = run(
        &request("release"),
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &Docs::default(),
            rankers: &[&scripted],
        },
        &shown,
        &mut kept,
    );
    let output = finished.result.expect("a run");
    assert_eq!(
        output,
        Output {
            hits: Vec::new(),
            searched: 0,
            stand: Stand::NoIndex,
            left_out: 0,
            skipped: Skipped::default(),
            scope: None,
        }
    );
    assert_eq!(scripted.calls.get(), 0);
    assert_eq!(fields(&kept)[2], "no-index");
}

#[test]
fn no_index_by_the_callers_word_is_the_no_index_answer_and_the_query_is_logged() {
    let mut log = shelf_of_six();
    log.changes_fail = true;
    let mut kept = Logged::default();
    let finished = run(
        &request("release"),
        &Reads {
            entries: &log,
            labels: &log,
        },
        &[Box::new(Never) as Box<dyn Reader>],
        &Bytes,
        Access::NoIndex,
        &shown,
        &mut kept,
    );
    assert_eq!(finished.unlogged, None);
    assert_eq!(
        finished.result.expect("a run"),
        Output {
            hits: Vec::new(),
            searched: 0,
            stand: Stand::NoIndex,
            left_out: 0,
            skipped: Skipped::default(),
            scope: None,
        }
    );
    assert!(log.reads.borrow().is_empty());
    assert_eq!(fields(&kept)[2], "no-index");
}

#[test]
fn an_index_of_another_manifest_is_refused_before_any_search() {
    let mut log = shelf_of_six();
    let index = index_of(&log);
    log.changes_fail = true;
    let scripted = Scripted::new(Vec::new());
    let mut other = request("release");
    other.manifest = Manifest::new(
        Size {
            tokens: 32,
            overlap: 8,
        },
        "bytes/1",
        "lines/1",
    );
    let mut kept = Logged::default();
    let finished = run(
        &other,
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &index,
            rankers: &[&scripted],
        },
        &shown,
        &mut kept,
    );
    let output = finished.result.expect("a run");
    assert_eq!(output.stand, Stand::OtherManifest);
    assert!(output.hits.is_empty());
    assert_eq!(output.searched, 0);
    assert_eq!(scripted.calls.get(), 0);
    assert_eq!(fields(&kept)[2], "other-manifest");
    other.manifest = Manifest::new(SIZE, "bytes/2", "lines/1");
    let again = run(
        &other,
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &index,
            rankers: &[&scripted],
        },
        &shown,
        &mut Logged::default(),
    );
    assert_eq!(again.result.expect("a run").stand, Stand::OtherManifest);
}

#[test]
fn an_index_behind_the_store_is_searched_and_says_by_how_many_entries() {
    let mut log = shelf_of_six();
    let index = index_of(&log);
    log.put(7, "note", &["a newer note about the bilbo release"]);
    log.append(2, &["and a second line about the same format"]);
    log.append(2, &["and a third one"]);
    let (output, _) = ask(&log, &index, &request("bilbo index format"));
    assert_eq!(output.stand, Stand::Behind(2));
    assert_eq!(output.searched, 6);
    assert_eq!(entries_of(&output)[0], id(2));
    assert_eq!(output.left_out, 0);
}

#[test]
fn an_index_of_another_catalog_is_searched_and_every_hit_checked() {
    let mut log = shelf_of_six();
    let index = index_of(&log);
    log.catalog = [9; 16];
    let (output, _) = ask(&log, &index, &request("bilbo index format"));
    assert_eq!(output.stand, Stand::Unknown);
    assert_eq!(entries_of(&output), [id(2)]);
    log.retouch(2, |held| {
        held.generations[0][0] = "decided something else entirely".to_owned();
    });
    let (output, _) = ask(&log, &index, &request("bilbo index format"));
    assert_eq!(output.stand, Stand::Unknown);
    assert!(output.hits.is_empty());
    assert_eq!(output.left_out, 1);
}

#[test]
fn a_session_that_grew_since_still_gives_its_indexed_chunks() {
    let mut log = Log::new();
    log.put(1, "session", &[RELEASE, "second record of this session"]);
    let index = index_of(&log);
    log.append(1, &["a third record that came later on"]);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(output.stand, Stand::Behind(1));
    assert_eq!(output.left_out, 0);
    assert_eq!(texts(&output), [RELEASE]);
}

#[test]
fn a_note_that_grew_in_place_is_left_out_not_shown_with_other_text() {
    let mut log = Log::new();
    log.put(
        1,
        "fenced",
        &["release checklist opens here", "and goes on"],
    );
    log.put(2, "fenced", &["release checklist of another note"]);
    let index = index_of(&log);
    log.append(1, &["!close"]);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(output.stand, Stand::Behind(1));
    assert_eq!(entries_of(&output), [id(2)]);
    assert_eq!(output.left_out, 1);
    assert!(output.hits.iter().all(|hit| !hit.text.contains("closed")));
}

#[test]
fn a_hit_of_a_superseded_generation_is_left_out() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    let index = index_of(&log);
    log.rewrite(1, &[RELEASE]);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(output.stand, Stand::Behind(1));
    assert_eq!(entries_of(&output), [id(2)]);
    assert_eq!(output.left_out, 1);
}

#[test]
fn a_hit_whose_entry_is_gone_or_whose_kind_no_reader_reads_is_left_out() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    log.put(3, "unread", &[RELEASE]);
    log.put(4, "note", &[RELEASE]);
    let mut index = Docs::default();
    let mut every = readers();
    every.push(Box::new(Lines("unread")));
    reindex::run(
        &work_request(),
        &log,
        &log,
        &every,
        &tokenizer(),
        &mut index,
    )
    .expect("a run");
    log.remove(1);
    let mut kept = Logged::default();
    let finished = ask_with(&log, &index, &request("release checklist"), &mut kept);
    let output = finished.result.expect("a run");
    assert_eq!(entries_of(&output), [id(2), id(4)]);
    assert_eq!(output.left_out, 2);
}

#[test]
fn an_entry_the_store_cannot_read_back_is_left_out_and_the_rest_come() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    log.put(3, "note", &[RELEASE]);
    let index = index_of(&log);
    log.retouch(2, |held| held.damaged = true);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(entries_of(&output), [id(1), id(3)]);
    assert_eq!(output.left_out, 1);
}

#[test]
fn left_out_hits_make_room_for_the_next_candidates() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    log.put(3, "note", &[RELEASE]);
    let index = index_of(&log);
    log.retouch(1, |held| held.damaged = true);
    let mut one_hit = request("release checklist");
    one_hit.budget = SHOWN_HEADER + RELEASE.len();
    let (output, _) = ask(&log, &index, &one_hit);
    assert_eq!(entries_of(&output), [id(2)]);
    assert_eq!(output.left_out, 1);
    assert!(!output.hits[0].cut);
}

#[test]
fn a_row_with_impossible_coordinates_is_left_out_and_the_rest_come() {
    let mut log = Log::new();
    for n in 1..=3 {
        log.put(n, "note", &[RELEASE]);
    }
    let mut index = index_of(&log);
    index.rows.sort();
    index.rows[0].first = u64::MAX;
    index.rows[0].count = 2;
    index.rows[0].text = "release checklist release checklist release".to_owned();
    let mut kept = Logged::default();
    let finished = ask_with(&log, &index, &request("release checklist"), &mut kept);
    let output = finished.result.expect("a run");
    assert_eq!(entries_of(&output), [id(2), id(3)]);
    assert_eq!(output.left_out, 1);
    assert_eq!(fields(&kept)[2], "hits");
}

#[test]
fn two_hits_of_one_entry_read_it_once() {
    let mut log = Log::new();
    log.put(
        1,
        "note",
        &[
            "release checklist first part of this note, long enough",
            "release checklist second part of this note, long enough",
        ],
    );
    let index = index_of(&log);
    assert_eq!(index.rows.len(), 2);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(output.hits.len(), 2);
    assert_eq!(log.reads.borrow().as_slice(), [id(1)]);
}

#[test]
fn only_as_many_entries_are_read_as_the_budget_needs() {
    let mut log = Log::new();
    for n in 1..=5 {
        log.put(n, "note", &[RELEASE]);
    }
    let index = index_of(&log);
    let mut two = request("release checklist");
    two.budget = 2 * (SHOWN_HEADER + RELEASE.len());
    let (output, _) = ask(&log, &index, &two);
    assert_eq!(output.hits.len(), 2);
    assert!(output.hits.iter().all(|hit| !hit.cut));
    assert_eq!(log.reads.borrow().len(), 2);
    log.reads.borrow_mut().clear();
    two.budget += SHOWN_HEADER + 5;
    let (output, _) = ask(&log, &index, &two);
    assert_eq!(output.hits.len(), 3);
    assert!(output.hits[2].cut);
    assert_eq!(log.reads.borrow().len(), 3);
}

#[test]
fn the_cap_and_the_candidates_bound_what_is_read() {
    let mut log = Log::new();
    log.put(
        1,
        "note",
        &[
            "release checklist one of three, long enough to stand alone",
            "release checklist two of three, long enough to stand alone",
            "release checklist three of three, long enough to stand alone",
        ],
    );
    log.put(2, "note", &[RELEASE]);
    let index = index_of(&log);
    assert_eq!(index.rows.len(), 4);
    let scripted = Scripted::new(candidates(&index, &[0, 1, 2, 3]));
    let mut bounded = request("release checklist");
    bounded.limits.per_entry = 1;
    bounded.candidates = 7;
    let mut kept = Logged::default();
    let finished = run(
        &bounded,
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &index,
            rankers: &[&scripted],
        },
        &shown,
        &mut kept,
    );
    let output = finished.result.expect("a run");
    assert_eq!(entries_of(&output), [id(1), id(2)]);
    assert_eq!(scripted.limits.borrow().as_slice(), [7]);
    assert_eq!(log.reads.borrow().len(), 2);
    bounded.limits.per_entry = 2;
    let finished = run(
        &bounded,
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &index,
            rankers: &[&scripted],
        },
        &shown,
        &mut Logged::default(),
    );
    assert_eq!(
        entries_of(&finished.result.expect("a run")),
        [id(1), id(1), id(2)]
    );
}

#[test]
fn two_rankers_are_fused_in_order() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    log.put(3, "note", &[RELEASE]);
    let index = index_of(&log);
    let first = Scripted::new(candidates(&index, &[2, 0]));
    let second = Scripted::new(candidates(&index, &[0, 1, 2]));
    let mut kept = Logged::default();
    let finished = run(
        &request("release"),
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &index,
            rankers: &[&first, &second],
        },
        &shown,
        &mut kept,
    );
    let output = finished.result.expect("a run");
    assert_eq!(entries_of(&output), [id(3), id(1), id(2)]);
    assert_eq!((first.calls.get(), second.calls.get()), (1, 1));
}

#[test]
fn every_query_is_logged_also_one_that_matches_nothing() {
    let log = shelf_of_six();
    let index = index_of(&log);
    let mut cases = Vec::new();

    let (_, kept) = ask(&log, &index, &request("bilbo index format"));
    cases.push(("hits", kept));
    let (_, kept) = ask(&log, &index, &request("zzzzzz"));
    cases.push(("nothing", kept));

    let mut kept = Logged::default();
    let finished = ask_with(&log, &Docs::default(), &request("index"), &mut kept);
    finished.result.expect("a run");
    cases.push(("no-index", kept));

    let mut other = request("index");
    other.chunk = Size {
        tokens: 128,
        overlap: 8,
    };
    other.manifest = Manifest::new(other.chunk, "bytes/1", "lines/1");
    let mut kept = Logged::default();
    ask_with(&log, &index, &other, &mut kept)
        .result
        .expect("a run");
    cases.push(("other-manifest", kept));

    let mut kept = Logged::default();
    let finished = run(
        &request("index"),
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Withheld,
        &shown,
        &mut kept,
    );
    finished.result.expect("a run");
    cases.push(("withheld", kept));

    for (ended, kept) in &cases {
        let fields = fields(kept);
        assert_eq!(&fields[2], ended);
        assert_eq!(fields[1], "1700000000");
    }
    assert_eq!(fields(&cases[0].1)[3], "6");
    assert_eq!(fields(&cases[1].1)[3], "6");
    assert_eq!(fields(&cases[2].1)[3], "0");
    assert!(fields(&cases[0].1)[7].starts_with(&id(2).to_string()));
    assert_eq!(fields(&cases[1].1)[7], "-");
}

#[test]
fn a_log_that_cannot_be_written_changes_neither_hits_nor_outcome() {
    let log = shelf_of_six();
    let index = index_of(&log);
    let asked = request("bilbo index format");
    let kept = ask_with(&log, &index, &asked, &mut Logged::default());
    let broken = ask_with(&log, &index, &asked, &mut Broken);
    assert_eq!(kept.unlogged, None);
    assert_eq!(broken.unlogged.as_deref(), Some("the disk is full"));
    assert_eq!(kept.result.expect("a run"), broken.result.expect("a run"));
}

#[test]
fn a_failed_append_is_reported_beside_an_index_error() {
    let log = shelf_of_six();
    let index = index_of(&log);
    let failing = Failing(&index, "postings");
    let finished = ask_with(&log, &failing, &request("bilbo index format"), &mut Broken);
    assert!(matches!(finished.result, Err(Error::Index(_))));
    assert_eq!(finished.unlogged.as_deref(), Some("the disk is full"));
}

#[test]
fn an_index_that_fails_at_any_read_ends_the_run_with_its_error_and_a_logged_line() {
    let log = shelf_of_six();
    let index = index_of(&log);
    for method in ["position", "totals", "postings", "holding", "found"] {
        let failing = Failing(&index, method);
        let mut kept = Logged::default();
        let finished = ask_with(
            &log,
            &failing,
            &of_kinds("bilbo index format", &["note"]),
            &mut kept,
        );
        let Err(Error::Index(error)) = finished.result else {
            panic!("{method}: an index error");
        };
        assert!(
            error.to_string().contains(&format!("{method} failed")),
            "{method}"
        );
        assert_eq!(finished.unlogged, None);
        let fields = fields(&kept);
        assert_eq!(fields[1], "1700000000", "{method}");
        assert_eq!(fields[2], "failed", "{method}");
        assert_eq!(fields[3], "0", "{method}");
        assert_eq!(fields[7], "-", "{method}");
        assert_eq!(fields[8], "bilbo index format", "{method}");
    }
    let mut unreadable = shelf_of_six();
    let index = index_of(&unreadable);
    unreadable.changes_fail = true;
    let mut kept = Logged::default();
    let finished = ask_with(&unreadable, &index, &request("index"), &mut kept);
    assert!(matches!(finished.result, Err(Error::Store(_))));
    assert_eq!(fields(&kept)[2], "failed");
}

#[test]
fn an_index_the_caller_withholds_is_not_read_and_the_query_is_logged() {
    let mut log = shelf_of_six();
    log.changes_fail = true;
    let scripted = Scripted::new(Vec::new());
    let mut kept = Logged::default();
    let finished = run(
        &request("release\tchecklist"),
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Withheld,
        &shown,
        &mut kept,
    );
    assert_eq!(finished.unlogged, None);
    assert_eq!(
        finished.result.expect("a run"),
        Output {
            hits: Vec::new(),
            searched: 0,
            stand: Stand::Withheld,
            left_out: 0,
            skipped: Skipped::default(),
            scope: None,
        }
    );
    assert_eq!(scripted.calls.get(), 0);
    assert!(log.reads.borrow().is_empty());
    let fields = fields(&kept);
    assert_eq!(fields[2], "withheld");
    assert_eq!(fields[8], "release\\tchecklist");
}

#[test]
fn the_same_store_and_query_give_the_same_hits_whatever_built_the_index() {
    let mut log = Log::new();
    log.put(1, "session", &[RELEASE, "second record of this session"]);
    log.put(2, "note", &["release notes for the bilbo checklist"]);
    log.put(3, "note", &[OTHER]);
    let mut grown = Docs::default();
    let steps = |log: &Log, index: &mut Docs| {
        let manifest = manifest();
        let Plan::Work(job) = update::plan(&manifest, 3, log, index).expect("a plan") else {
            return;
        };
        update::apply(
            job,
            &work_request(),
            log,
            log,
            &readers(),
            &tokenizer(),
            index,
        )
        .expect("a run");
    };
    steps(&log, &mut grown);
    log.append(1, &["release checklist again, a third record here"]);
    steps(&log, &mut grown);
    log.put(4, "memo", &["checklist for the release of memo kind"]);
    log.rewrite(3, &["grocery list release checklist bread"]);
    steps(&log, &mut grown);
    log.append(2, &["one more line on the release checklist"]);
    steps(&log, &mut grown);
    let rebuilt = index_of(&log);
    assert_eq!(grown.sorted(), rebuilt.sorted());
    for text in [
        "release checklist",
        "bilbo",
        "grocery bread",
        "memo kind release",
    ] {
        let (from_grown, _) = ask(&log, &grown, &request(text));
        let (from_rebuilt, _) = ask(&log, &rebuilt, &request(text));
        assert_eq!(from_grown, from_rebuilt, "{text}");
        assert!(!from_grown.hits.is_empty(), "{text}");
    }
}

#[test]
fn the_use_case_reads_no_clock() {
    let log = shelf_of_six();
    let index = index_of(&log);
    let mut early = request("bilbo index format");
    early.at = at(42);
    let (_, kept) = ask(&log, &index, &early);
    assert_eq!(fields(&kept)[1], "42");
    let mut late = request("bilbo index format");
    late.at = at(4_000_000_000);
    let (_, kept) = ask(&log, &index, &late);
    assert_eq!(fields(&kept)[1], "4000000000");
}

#[test]
fn a_portuguese_query_ranks_by_its_content_words() {
    let mut log = Log::new();
    log.put(1, "note", &["a tarifa da luz subiu"]);
    log.put(2, "note", &["que pelo nas que pelo nas"]);
    log.put(3, "note", &[OTHER]);
    let index = index_of(&log);
    let hits = |languages: &[Language], detect: Option<&dyn Detect>| {
        let bm25 = Bm25 {
            index: &index,
            k1: 1.2,
            b: 0.75,
            omit: Omit {
                words: &[],
                languages,
                detect,
            },
            batch: 4096,
        };
        let mut kept = Logged::default();
        let finished = run(
            &request("que pelo nas sobre a tarifa luz"),
            &Reads {
                entries: &log,
                labels: &log,
            },
            &readers(),
            &Bytes,
            Access::Read {
                index: &index,
                rankers: &[&bm25],
            },
            &shown,
            &mut kept,
        );
        entries_of(&finished.result.expect("a run"))
    };
    let portuguese = [Language::Portuguese];
    let says = Says(Some(Language::Portuguese));
    assert!(hits(&portuguese, None).contains(&id(2)));
    assert_eq!(hits(&portuguese, Some(&says)), [id(1)]);
    assert!(hits(&portuguese, Some(&Says(None))).contains(&id(2)));
    assert!(hits(&[], Some(&says)).contains(&id(2)));
}

fn passed(copies: u64, parts: u64, capped: u64) -> Skipped {
    Skipped {
        copies,
        parts,
        capped,
    }
}

fn words(prefix: &str, count: usize) -> String {
    (1..=count)
        .map(|n| {
            if n % 3 == 1 {
                format!("release {prefix}{n:02}")
            } else {
                format!("{prefix}{n:02}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn what_was_passed_over_is_counted_by_reason() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    log.put(3, "note", &[&words("a", 20)]);
    let index = index_of(&log);
    assert_eq!(
        index.rows.iter().filter(|row| row.entry == id(3)).count(),
        3
    );
    let mut limited = request("release checklist");
    limited.limits = PASS;
    let (output, _) = ask(&log, &index, &limited);
    assert_eq!(output.skipped, passed(1, 1, 0));
    assert_eq!(output.hits.len(), 3);
}

#[test]
fn an_equal_chunk_is_passed_over_and_its_entry_never_read() {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    let index = index_of(&log);
    let mut limited = request("release checklist");
    limited.limits = PASS;
    let (output, _) = ask(&log, &index, &limited);
    assert_eq!(entries_of(&output), [id(1)]);
    assert_eq!(log.reads.borrow().as_slice(), [id(1)]);
    assert_eq!(output.skipped.copies, 1);
}

#[test]
fn a_hit_passed_over_makes_room_for_the_next_candidate() {
    let mut log = Log::new();
    let later = [
        "release checklist for the quarterly planning review meeting agenda",
        "release checklist for the yearly budget review meeting minutes",
    ];
    for n in 1..=3 {
        log.put(n, "note", &[RELEASE]);
    }
    log.put(4, "note", &[later[0]]);
    log.put(5, "note", &[later[1]]);
    let index = index_of(&log);
    let mut limited = request("release checklist");
    limited.limits = PASS;
    limited.budget = 3 * SHOWN_HEADER + RELEASE.len() + later[0].len() + later[1].len();
    let (output, _) = ask(&log, &index, &limited);
    let mut found = entries_of(&output);
    found.sort();
    assert_eq!(found, [id(1), id(4), id(5)]);
    assert!(output.hits.iter().all(|hit| !hit.cut));
    assert_eq!(output.skipped.copies, 2);
}

#[test]
fn a_cut_hit_that_repeats_a_hit_above_is_not_printed_and_the_next_is_tried() {
    let a = "a1 b2 c3 d4 e5 f6";
    let b = "a1 b2 c3 d4 e5 f6 x1 y1 z1 x2 y2 z2 x3 y3 z3 x4 y4 z4 x5 y5";
    let c = "u1 v2 w3 q4 r5 s6";
    let mut log = Log::new();
    log.put(1, "note", &[a]);
    log.put(2, "note", &[b]);
    log.put(3, "note", &[c]);
    let index = index_of(&log);
    let scripted = Scripted::new(candidates(&index, &[0, 1, 2]));
    let mut limited = request("unused");
    limited.limits = PASS;
    limited.budget = 2 * SHOWN_HEADER + a.len() + 24;
    let finished = run(
        &limited,
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &index,
            rankers: &[&scripted],
        },
        &shown,
        &mut Logged::default(),
    );
    let output = finished.result.expect("a run");
    assert_eq!(entries_of(&output), [id(1), id(3)]);
    assert_eq!(output.skipped, passed(1, 0, 0));
}

#[test]
fn no_record_is_in_more_than_two_hits_of_its_entry() {
    let long = words("l", 22);
    let records = [words("a", 9), long, words("c", 9)];
    let mut log = Log::new();
    log.put(
        1,
        "joined",
        &[
            records[0].as_str(),
            records[1].as_str(),
            records[2].as_str(),
        ],
    );
    let index = index_of(&log);
    let ranges: Vec<(u64, u64)> = index
        .rows
        .iter()
        .map(|row| (row.first, row.first + row.count - 1))
        .collect();
    assert!(
        ranges.contains(&(1, 2)) && ranges.contains(&(2, 2)) && ranges.contains(&(2, 3)),
        "{ranges:?}"
    );
    let mut limited = request("release");
    limited.limits = PASS;
    let (output, _) = ask(&log, &index, &limited);
    for record in 1..=3 {
        let holding = output
            .hits
            .iter()
            .filter(|hit| {
                let range = hit.found.extent.records;
                (range.first..range.first + range.count).contains(&record)
            })
            .count();
        assert!(holding <= 2, "record {record} is in {holding} hits");
    }
    assert!(output.skipped.parts > 0);
}

#[test]
fn a_hit_whose_short_id_another_entry_shares_is_marked() {
    let mut first = [0u8; 16];
    first[0] = 1;
    first[15] = 5;
    let mut second = first;
    second[0] = 2;
    let mut log = Log::new();
    log.put_id(Id::from_bytes(first), "note", &["release checklist one"]);
    log.put_id(Id::from_bytes(second), "note", &["release checklist two"]);
    log.put(6, "note", &["release checklist three"]);
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(output.hits.len(), 3);
    for hit in &output.hits {
        let shares = hit.found.entry != id(6);
        assert_eq!(hit.shared_short, shares, "{}", hit.found.entry);
    }
}

#[test]
fn the_short_ids_are_looked_up_once_a_recall_and_never_with_no_candidate() {
    let mut log = Log::new();
    for n in 1..=5u8 {
        let records: Vec<String> = (0..4)
            .map(|k| format!("release checklist {}", words(&format!("e{n}k{k}"), 4)))
            .collect();
        let records: Vec<&str> = records.iter().map(String::as_str).collect();
        log.put(n, "note", &records);
    }
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(output.hits.len(), 20);
    assert_eq!(log.shorts.borrow().as_slice(), [5]);
    log.shorts.borrow_mut().clear();
    let (output, _) = ask(&log, &index, &request("zzzzqqqq"));
    assert!(output.hits.is_empty());
    assert!(log.shorts.borrow().is_empty());
}

fn scoped(text: &str, values: &[&str]) -> Request {
    let mut request = request(text);
    request.scope = values.iter().map(|value| (*value).to_owned()).collect();
    request
}

fn shelf_of_places() -> Log {
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.put(2, "note", &[RELEASE]);
    log.put(3, "session", &[RELEASE]);
    log.put(4, "memo", &[RELEASE]);
    log.label(1, "file:/home/me/src/shed");
    log.label(2, "file:/home/me/src/garden");
    log.label(3, "file:/home/me/src/shed");
    log
}

#[test]
fn a_scope_narrows_the_hits_to_the_entries_it_names() {
    let log = shelf_of_places();
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &scoped("release checklist", &["garden"]));
    assert_eq!(entries_of(&output), [id(2)]);
    let (output, _) = ask(
        &log,
        &index,
        &scoped("release checklist", &["shed", "garden"]),
    );
    let mut found = entries_of(&output);
    found.sort();
    assert_eq!(found, [id(1), id(2), id(3)]);
    let (output, _) = ask(&log, &index, &request("release checklist"));
    assert_eq!(output.hits.len(), 4);
    assert_eq!(output.scope, None);
}

#[test]
fn a_scope_that_names_nothing_searches_nothing() {
    let log = shelf_of_places();
    let index = index_of(&log);
    let scripted = Scripted::new(candidates(&index, &[0, 1]));
    let mut kept = Logged::default();
    let finished = run(
        &scoped("release", &["greenhouse"]),
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::Read {
            index: &index,
            rankers: &[&scripted],
        },
        &shown,
        &mut kept,
    );
    let output = finished.result.expect("a run");
    assert_eq!(scripted.calls.get(), 0);
    assert!(output.hits.is_empty());
    assert_eq!(output.searched, 0);
    assert_eq!(
        output.scope,
        Some(Scoped {
            kept: 0,
            unnamed: vec![("greenhouse".to_owned(), Unnamed::Nothing)],
            worded: true,
            best: 0,
            outside: BTreeMap::new(),
        })
    );
    assert_eq!(fields(&kept)[2], "nothing");
}

#[test]
fn a_label_is_found_by_its_first_64_words_and_a_longer_value_names_nothing() {
    let words = |count: usize| {
        (0..count)
            .map(|n| format!("w{n}"))
            .collect::<Vec<_>>()
            .join("/")
    };
    let long = words(70);
    let mut log = Log::new();
    log.put(1, "note", &[RELEASE]);
    log.label(1, &long);
    let index = index_of(&log);
    assert_eq!(
        log.filed.borrow()[&id(1)],
        [(long.clone(), label_words(&long)[..MAX_LABEL_WORDS].to_vec())]
    );
    let (output, _) = ask(&log, &index, &scoped("release checklist", &[&long]));
    assert!(output.hits.is_empty());
    assert_eq!(
        output.scope,
        Some(Scoped {
            kept: 0,
            unnamed: vec![(long.clone(), Unnamed::Nothing)],
            worded: true,
            best: 0,
            outside: BTreeMap::new(),
        })
    );
    assert_eq!(log.asked_words.borrow().last().map(Vec::len), Some(70));
    let (output, _) = ask(
        &log,
        &index,
        &scoped("release checklist", &[&words(MAX_LABEL_WORDS)]),
    );
    assert_eq!(entries_of(&output), [id(1)]);
}

#[test]
fn the_best_matches_outside_a_scope_are_counted_by_kind() {
    let log = shelf_of_places();
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &scoped("release checklist", &["garden"]));
    let scope = output.scope.expect("a scope");
    assert_eq!((scope.kept, scope.best), (1, 4));
    assert_eq!(
        scope.outside,
        BTreeMap::from([
            ("memo".to_owned(), 1),
            ("note".to_owned(), 1),
            ("session".to_owned(), 1)
        ])
    );
    let (output, _) = ask(&log, &index, &scoped("release checklist", &["shed"]));
    let scope = output.scope.expect("a scope");
    assert_eq!((scope.kept, scope.best), (2, 4));
    assert_eq!(
        scope.outside,
        BTreeMap::from([("memo".to_owned(), 1), ("note".to_owned(), 1)])
    );
}

#[test]
fn a_kind_narrows_a_scope() {
    let log = shelf_of_places();
    let index = index_of(&log);
    let mut both = scoped("release checklist", &["shed", &id(4).to_string()]);
    both.query.kinds = vec!["note".to_owned()];
    let (output, _) = ask(&log, &index, &both);
    assert_eq!(entries_of(&output), [id(1)]);
    let scope = output.scope.expect("a scope");
    assert_eq!((scope.kept, scope.best), (3, 2));
    assert_eq!(scope.outside, BTreeMap::from([("note".to_owned(), 1)]));
}

#[test]
fn an_id_takes_a_sessions_subagents() {
    let mut log = shelf_of_places();
    log.put(5, "session", &[RELEASE]);
    log.put(6, "session", &[RELEASE]);
    log.belong(5, 3);
    log.belong(6, 5);
    let index = index_of(&log);
    let (output, _) = ask(
        &log,
        &index,
        &scoped("release checklist", &[&id(3).to_string()]),
    );
    let mut found = entries_of(&output);
    found.sort();
    assert_eq!(found, [id(3), id(5), id(6)]);
    assert_eq!(log.asked_belonging.borrow().as_slice(), [vec![id(3)]]);
}

#[test]
fn a_scope_says_whether_words_were_asked() {
    let log = shelf_of_places();
    let index = index_of(&log);
    let (output, _) = ask(&log, &index, &scoped("release checklist", &["garden"]));
    assert!(output.scope.expect("a scope").worded);
    let (output, _) = ask(
        &log,
        &index,
        &scoped("release checklist", &[&id(2).to_string()]),
    );
    assert!(!output.scope.expect("a scope").worded);
}

#[test]
fn the_query_log_holds_the_values_and_how_many_entries_they_named() {
    let log = shelf_of_places();
    let index = index_of(&log);
    let (_, kept) = ask(
        &log,
        &index,
        &scoped("release checklist", &["shed", "garden"]),
    );
    let line = fields(&kept);
    assert_eq!(line[5], "shed,garden");
    assert_eq!(line[6], "3");
    let (_, kept) = ask(&log, &index, &scoped("release checklist", &["nowhere"]));
    let line = fields(&kept);
    assert_eq!(
        (line[2].as_str(), line[5].as_str(), line[6].as_str()),
        ("nothing", "nowhere", "0")
    );
    let (_, kept) = ask(&log, &index, &request("release checklist"));
    let line = fields(&kept);
    assert_eq!((line[5].as_str(), line[6].as_str()), ("-", "-"));
    let mut kept = Logged::default();
    let finished = run(
        &scoped("release checklist", &["shed", "garden"]),
        &Reads {
            entries: &log,
            labels: &log,
        },
        &readers(),
        &Bytes,
        Access::NoIndex,
        &shown,
        &mut kept,
    );
    assert!(finished.result.is_ok());
    let line = fields(&kept);
    assert_eq!(
        (line[2].as_str(), line[5].as_str(), line[6].as_str()),
        ("no-index", "shed,garden", "-")
    );
}

#[test]
fn a_store_that_fails_while_a_scope_resolves_ends_the_run_logged() {
    let log = shelf_of_places();
    let index = index_of(&log);
    log.labels_fail.set(true);
    let mut kept = Logged::default();
    let finished = ask_with(
        &log,
        &index,
        &scoped("release checklist", &["shed"]),
        &mut kept,
    );
    assert!(matches!(finished.result, Err(Error::Store(_))));
    let line = fields(&kept);
    assert_eq!(
        (line[2].as_str(), line[5].as_str(), line[6].as_str()),
        ("failed", "shed", "-")
    );
}
