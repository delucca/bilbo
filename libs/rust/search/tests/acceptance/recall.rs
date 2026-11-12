use bilbo_kinds::reader::Reader;
use bilbo_search::bm25::Bm25;
use bilbo_search::chunk::Size;
use bilbo_search::lookup::Lookup;
use bilbo_search::position::Manifest;
use bilbo_search::rank::{Candidate, Query};
use bilbo_search::recall::hit::Hit;
use bilbo_search::recall::log::QueryLog;
use bilbo_search::recall::{Access, Error, Finished, Output, Request, Stand, run};
use bilbo_search::reindex;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_search::update::{self, Plan};
use bilbo_search::work::Request as Work;
use bilbo_store::id::Id;

use crate::fakes::{
    Broken, Bytes, Dated, Docs, Failing, Fenced, Lines, Log, Logged, Never, Scripted, at, id,
};

const SIZE: Size = Size {
    tokens: 64,
    overlap: 8,
};

const SHOWN_HEADER: usize = 4 + 26 + 1;

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
    ]
}

fn work_request() -> Work {
    Work {
        chunk: SIZE,
        render: "lines/1".to_owned(),
        workers: 1,
    }
}

fn index_of(log: &Log) -> Docs {
    let mut index = Docs::default();
    reindex::run(&work_request(), log, &readers(), &tokenizer(), &mut index).expect("a run");
    log.reads.borrow_mut().clear();
    index
}

fn request(text: &str) -> Request {
    Request {
        query: Query {
            text: text.to_owned(),
            kinds: Vec::new(),
        },
        manifest: manifest(),
        chunk: SIZE,
        candidates: 50,
        cap: 0,
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
        dropped: &[],
        batch: 4096,
    };
    run(
        request,
        log,
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
    assert_eq!(fields.len(), 7, "{line:?}");
    assert_eq!(fields[0], "q1");
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
                left_out: 0
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
        &log,
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
            left_out: 0
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
        &log,
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
            left_out: 0
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
        &log,
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
        &log,
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
    reindex::run(&work_request(), &log, &every, &tokenizer(), &mut index).expect("a run");
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
    bounded.cap = 1;
    bounded.candidates = 7;
    let mut kept = Logged::default();
    let finished = run(
        &bounded,
        &log,
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
    bounded.cap = 2;
    let finished = run(
        &bounded,
        &log,
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
        &log,
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
        &log,
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
    assert!(fields(&cases[0].1)[5].starts_with(&id(2).to_string()));
    assert_eq!(fields(&cases[1].1)[5], "-");
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
        assert_eq!(fields[5], "-", "{method}");
        assert_eq!(fields[6], "bilbo index format", "{method}");
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
        &log,
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
            left_out: 0
        }
    );
    assert_eq!(scripted.calls.get(), 0);
    assert!(log.reads.borrow().is_empty());
    let fields = fields(&kept);
    assert_eq!(fields[2], "withheld");
    assert_eq!(fields[6], "release\\tchecklist");
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
        update::apply(job, &work_request(), log, &readers(), &tokenizer(), index).expect("a run");
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
