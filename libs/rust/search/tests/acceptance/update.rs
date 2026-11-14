use bilbo_kinds::reader::Reader;
use bilbo_search::chunk::Size;
use bilbo_search::position::Manifest;
use bilbo_search::reindex;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_search::update::{Output, Plan, Reason, apply, plan};
use bilbo_search::work::{Error, Request};
use bilbo_store::entries::Cursor;
use bilbo_store::id::Id;

use crate::fakes::{
    Bytes, CATALOG, Docs, Fault, Foldered, Lines, Log, Row, Whole, id, named, unread, wide,
};
use bilbo_search::analyze::label_words;
use bilbo_store::labels::Labels;

const SMALL: Size = Size {
    tokens: 12,
    overlap: 2,
};

const TRIES: u8 = 3;

fn request_of(chunk: Size, workers: usize) -> Request {
    Request {
        chunk,
        render: "lines/1".to_owned(),
        workers,
        label_batch: 512,
    }
}

fn request() -> Request {
    request_of(SMALL, 1)
}

fn manifest_of(request: &Request) -> Manifest {
    Manifest::new(request.chunk, "bytes/1", &request.render)
}

fn tokenizer() -> Tokenizer {
    Tokenizer {
        tokens: Box::new(Bytes),
        id: "bytes/1",
    }
}

fn readers() -> Vec<Box<dyn Reader>> {
    vec![
        Box::new(Foldered("lines")),
        Box::new(Lines("notes")),
        Box::new(Whole("plain")),
    ]
}

fn plan_of(log: &Log, index: &mut Docs, tries: u8) -> Plan {
    plan(&manifest_of(&request()), tries, log, index).expect("a plan")
}

fn keep_with(
    request: &Request,
    log: &Log,
    index: &mut Docs,
    tries: u8,
) -> Result<Option<Output>, Error> {
    match plan(&manifest_of(request), tries, log, index)? {
        Plan::Work(work) => {
            apply(work, request, log, log, &readers(), &tokenizer(), index).map(Some)
        }
        Plan::Current => Ok(None),
    }
}

fn keep(log: &Log, index: &mut Docs) -> Output {
    keep_with(&request(), log, index, TRIES)
        .expect("a run")
        .expect("work")
}

fn rebuilt_with(request: &Request, log: &Log) -> Vec<Row> {
    let mut fresh = Docs::default();
    reindex::run(request, log, log, &readers(), &tokenizer(), &mut fresh).expect("a rebuild");
    fresh.sorted()
}

fn rebuilt(log: &Log) -> Vec<Row> {
    rebuilt_with(&request(), log)
}

fn failed_of(index: &Docs) -> Vec<(Id, u8)> {
    index.position.clone().expect("a commit").failed
}

#[test]
fn an_index_at_the_stores_cursor_is_left_alone() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    let (writes, position) = (index.writes, index.position.clone());
    assert_eq!(plan_of(&log, &mut index, TRIES), Plan::Current);
    assert_eq!(plan_of(&log, &mut index, 0), Plan::Current);
    assert!(
        keep_with(&request(), &log, &mut index, TRIES)
            .expect("a run")
            .is_none()
    );
    assert_eq!((index.writes, index.position.clone()), (writes, position));
}

#[test]
fn an_index_never_made_is_planned_as_a_rebuild_and_planning_writes_nothing() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    let mut index = Docs::default();
    let Plan::Work(work) = plan_of(&log, &mut index, TRIES) else {
        panic!("work");
    };
    assert_eq!(work.rebuilds(), Some(Reason::Unmade));
    assert_eq!(index.writes, 0);
}

#[test]
fn a_grown_session_is_indexed_from_the_chunk_that_holds_its_last_passage() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb", "ccccc"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    assert_eq!(index.rows.len(), 2);
    log.append(1, &["ddddd"]);
    let output = keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 3)]);
    assert_eq!(
        (output.entries, output.chunks, output.rebuilt),
        (1, 1, false)
    );
    assert_eq!(index.sorted(), rebuilt(&log));
    assert_eq!(
        index.position.clone().expect("a commit").cursor,
        log.cursor()
    );
}

#[test]
fn a_passage_that_grows_the_entry_over_the_size_gets_windows_and_the_packed_chunks_before_it_stay()
{
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb", "ccccc"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.append(1, &["dddddddddddddddddddddddddddddd"]);
    let output = keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 3)]);
    assert!(output.chunks > 2, "{output:?}");
    assert_eq!(index.sorted(), rebuilt(&log));
    assert!(
        index
            .rows
            .iter()
            .any(|row| row.first == 1 && row.text == "aaaaa\nbbbbb")
    );
}

#[test]
fn a_windowed_last_passage_is_done_again_from_its_first_window() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "dddddddddddddddddddddddddddddd"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.append(1, &["eeeee"]);
    keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 2)]);
    assert_eq!(index.sorted(), rebuilt(&log));
}

struct Random(u64);

impl Random {
    fn next(&mut self, below: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % below as u64) as usize
    }
}

const WORDS: [&str; 10] = [
    "a",
    "bb",
    "cccc",
    "dddddd",
    "eeeeeeee",
    "-",
    "ffffffffffffffffffff",
    "gggggggggggggggggggggggggggggggggggg",
    "@50 hh",
    "@70 iiiii",
];

#[test]
fn growth_in_any_number_of_steps_gives_the_documents_of_a_rebuild() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for case in 0..300 {
        let mut log = Log::new();
        let mut index = Docs::default();
        let entries = 1 + random.next(3) as u8;
        let kinds = ["lines", "plain", "notes"];
        for n in 1..=entries {
            log.put(n, kinds[random.next(3)], &[]);
        }
        for step in 0..(1 + random.next(5)) {
            for n in 1..=entries {
                if random.next(3) == 0 {
                    continue;
                }
                let records: Vec<&str> = (0..random.next(5))
                    .map(|_| WORDS[random.next(WORDS.len())])
                    .collect();
                if random.next(8) == 0 && step > 0 {
                    log.rewrite(n, &records);
                } else {
                    log.append(n, &records);
                }
            }
            keep_with(&request(), &log, &mut index, TRIES).expect("a run");
            assert_eq!(index.sorted(), rebuilt(&log), "case {case} step {step}");
        }
    }
}

#[test]
fn a_new_generation_replaces_every_document_of_the_old_one() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb", "ccccc"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.rewrite(1, &["new one", "new two"]);
    keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 1)]);
    assert!(index.rows.iter().all(|row| row.generation == 2));
    assert_eq!(index.sorted(), rebuilt(&log));
}

#[test]
fn a_meta_change_does_the_whole_entry_again() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb", "ccccc"]);
    log.put(2, "lines", &["xxxxx"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.retouch(1, |held| held.id_label = Some(id(2)));
    keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 1)]);
    assert_eq!(index.sorted(), rebuilt(&log));

    index.deletes.clear();
    log.retouch(2, |held| held.captured = crate::fakes::at(5));
    keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(2), 1)]);
    assert_eq!(index.sorted(), rebuilt(&log));
}

#[test]
fn an_entry_whose_kind_no_reader_reads_any_more_loses_its_documents_and_is_counted_unread() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb"]);
    log.put(2, "lines", &["xxxxx"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.retouch(1, |held| held.kind = "pdf".to_owned());
    let output = keep(&log, &mut index);
    assert_eq!(output.unread, unread(&[("pdf", 1)]));
    assert_eq!(index.deletes, [(id(1), 1)]);
    assert!(index.rows.iter().all(|row| row.entry == id(2)));
    assert_eq!(index.sorted(), rebuilt(&log));
}

#[test]
fn an_entry_whose_kind_gains_a_reader_is_indexed() {
    let mut log = Log::new();
    log.put(1, "pdf", &["aaaaa"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    assert!(index.rows.is_empty());
    log.retouch(1, |held| held.kind = "lines".to_owned());
    let output = keep(&log, &mut index);
    assert_eq!((output.entries, output.chunks), (1, 1));
    assert_eq!(index.sorted(), rebuilt(&log));
    assert_eq!(index.rows.len(), 1);
}

#[test]
fn a_failed_entry_that_becomes_unread_leaves_the_list() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    log.entries[0].damaged = true;
    let mut index = Docs::default();
    keep(&log, &mut index);
    assert_eq!(failed_of(&index), [(id(1), 1)]);
    log.retouch(1, |held| held.kind = "pdf".to_owned());
    keep(&log, &mut index);
    assert_eq!(failed_of(&index), []);
    assert_eq!(plan_of(&log, &mut index, TRIES), Plan::Current);
}

#[test]
fn an_entry_the_store_no_longer_has_loses_its_documents() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb"]);
    log.put(2, "lines", &["xxxxx"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.remove(1);
    keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 1)]);
    assert_eq!(index.sorted(), rebuilt(&log));
}

#[test]
fn a_kind_that_does_not_promise_appends_only_does_the_whole_entry_again() {
    let mut log = Log::new();
    log.put(1, "plain", &["aaaaa", "bbbbb", "ccccc"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.append(1, &["ddddd"]);
    keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 1)]);
    assert_eq!(index.sorted(), rebuilt(&log));
}

#[test]
fn an_entry_with_no_document_is_done_whole() {
    let mut log = Log::new();
    log.put(1, "lines", &["-"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    assert!(index.rows.is_empty());
    log.append(1, &["text"]);
    keep(&log, &mut index);
    assert_eq!(index.deletes, [(id(1), 1)]);
    assert_eq!(index.sorted(), rebuilt(&log));
    assert_eq!(index.rows.len(), 1);
}

#[test]
fn an_entry_that_fails_loses_its_documents_and_is_listed_and_the_cursor_moves() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb"]);
    log.put(2, "lines", &["xxxxx"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.append(1, &["ccccc"]);
    log.entries[0].damaged = true;
    log.append(2, &["yyyyy"]);
    let output = keep(&log, &mut index);
    assert_eq!(output.failed.len(), 1);
    assert_eq!(output.failed[0].0, id(1));
    assert!(index.rows.iter().all(|row| row.entry == id(2)));
    assert_eq!(failed_of(&index), [(id(1), 1)]);
    assert_eq!(
        index.position.clone().expect("a commit").cursor,
        log.cursor()
    );
    assert_eq!(index.deletes[0], (id(1), 1));
}

#[test]
fn a_changed_manifest_rebuilds_in_full_from_the_first_change() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb", "ccccc"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.append(1, &["ddddd"]);
    let wider = request_of(
        Size {
            tokens: 20,
            overlap: 2,
        },
        1,
    );
    let before = (index.writes, index.sorted());
    let Plan::Work(work) = plan(&manifest_of(&wider), TRIES, &log, &mut index).expect("a plan")
    else {
        panic!("work");
    };
    assert_eq!(work.rebuilds(), Some(Reason::Manifest));
    assert_eq!((index.writes, index.sorted()), before);
    let output = keep_with(&wider, &log, &mut index, TRIES)
        .expect("a run")
        .expect("work");
    assert!(output.rebuilt);
    assert!(index.deletes.is_empty());
    assert_eq!(index.sorted(), rebuilt_with(&wider, &log));
    assert_eq!(
        index.position.clone().expect("a commit").cursor,
        log.cursor()
    );
}

#[test]
fn a_cursor_of_another_catalog_rebuilds_in_full() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.catalog = [9; 16];
    log.append(1, &["ccccc"]);
    let before = (index.writes, index.sorted());
    let Plan::Work(work) = plan_of(&log, &mut index, TRIES) else {
        panic!("work");
    };
    assert_eq!(work.rebuilds(), Some(Reason::Catalog));
    assert_eq!((index.writes, index.sorted()), before);
    let output = keep(&log, &mut index);
    assert!(output.rebuilt);
    assert_eq!(index.sorted(), rebuilt(&log));
    assert_eq!(
        index.position.clone().expect("a commit").cursor.catalog(),
        [9; 16]
    );
}

#[test]
fn an_index_ahead_of_the_store_is_rebuilt() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    log.append(1, &["bbbbb"]);
    log.append(1, &["ccccc"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    let mut older = Log::new();
    older.put(1, "lines", &["aaaaa"]);
    assert_eq!(older.cursor(), Cursor::at(CATALOG, 2));
    let Plan::Work(work) = plan_of(&older, &mut index, TRIES) else {
        panic!("work");
    };
    assert_eq!(work.rebuilds(), Some(Reason::Catalog));
    let output = keep(&older, &mut index);
    assert!(output.rebuilt);
    assert_eq!(index.sorted(), rebuilt(&older));
}

#[test]
fn an_error_after_adds_rolls_back_and_the_next_run_does_the_work() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb"]);
    log.put(2, "lines", &["xxxxx"]);
    let mut index = Docs::default();
    keep(&log, &mut index);
    let (rows, position) = (index.sorted(), index.position.clone());
    log.append(1, &["ccccc"]);
    log.append(2, &["yyyyy"]);
    index.fail_add = Some(3);
    let error = keep_with(&request(), &log, &mut index, TRIES).expect_err("a failure");
    assert!(matches!(error, Error::Index(_)), "{error:?}");
    assert_eq!(index.rolled_back, 1);
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    index.fail_add = None;
    keep(&log, &mut index);
    assert_eq!(index.sorted(), rebuilt(&log));
}

#[test]
fn an_entry_left_out_is_tried_again_by_the_next_two_invocations_and_then_left() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    log.put(2, "lines", &["xxxxx"]);
    log.entries[0].damaged = true;
    let mut index = Docs::default();
    keep(&log, &mut index);
    assert_eq!(failed_of(&index), [(id(1), 1)]);
    for tries in [2, 3] {
        let output = keep(&log, &mut index);
        assert_eq!(output.failed.len(), 1, "try {tries}");
        assert_eq!(failed_of(&index), [(id(1), tries)]);
    }
    assert_eq!(plan_of(&log, &mut index, TRIES), Plan::Current);
    log.append(1, &["bbbbb"]);
    let output = keep(&log, &mut index);
    assert_eq!(output.failed.len(), 1);
    assert_eq!(failed_of(&index), [(id(1), 4)]);
}

#[test]
fn an_entry_is_tried_as_many_runs_as_the_tries_say_and_then_left() {
    for (tries, tried) in [(1, 1), (5, 5)] {
        let mut log = Log::new();
        log.put(1, "lines", &["aaaaa"]);
        log.put(2, "lines", &["xxxxx"]);
        log.entries[0].damaged = true;
        let mut index = Docs::default();
        keep_with(&request(), &log, &mut index, tries).expect("a run");
        for run in 2..=tried {
            let output = keep_with(&request(), &log, &mut index, tries)
                .expect("a run")
                .expect("work");
            assert_eq!(output.failed.len(), 1, "tries {tries}, run {run}");
        }
        assert_eq!(failed_of(&index), [(id(1), tried)], "tries {tries}");
        assert_eq!(plan_of(&log, &mut index, tries), Plan::Current);
    }
}

#[test]
fn a_failed_entry_that_is_indexed_at_a_retry_leaves_the_list() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    log.entries[0].damaged = true;
    let mut index = Docs::default();
    keep(&log, &mut index);
    log.entries[0].damaged = false;
    let output = keep(&log, &mut index);
    assert_eq!((output.entries, output.chunks), (1, 1));
    assert_eq!(failed_of(&index), []);
    assert_eq!(index.sorted(), rebuilt(&log));
}

#[test]
fn a_pass_that_may_not_retry_keeps_the_list_as_it_is() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    log.put(2, "lines", &["xxxxx"]);
    log.entries[0].damaged = true;
    let mut index = Docs::default();
    keep(&log, &mut index);
    let no_retry = 0;
    assert_eq!(plan_of(&log, &mut index, no_retry), Plan::Current);
    log.append(2, &["yyyyy"]);
    keep_with(&request(), &log, &mut index, no_retry)
        .expect("a run")
        .expect("work");
    assert_eq!(failed_of(&index), [(id(1), 1)]);
    assert_eq!(
        index.position.clone().expect("a commit").cursor,
        log.cursor()
    );
}

#[test]
fn a_passage_over_the_bound_that_follows_new_records_is_windowed_as_a_rebuild_windows_it() {
    let big = {
        let lines: Vec<String> = (0..45_000)
            .map(|n| format!("some short line {n:07} of a long record"))
            .collect();
        lines.join("\n")
    };
    assert!(big.len() > 1 << 20);
    let request = request_of(
        Size {
            tokens: 4096,
            overlap: 64,
        },
        2,
    );
    let mut log = Log::new();
    log.put(1, "lines", &["first"]);
    let mut index = Docs::default();
    keep_with(&request, &log, &mut index, TRIES).expect("a run");
    log.append(1, &[big.as_str()]);
    keep_with(&request, &log, &mut index, TRIES).expect("a run");
    log.append(1, &["last"]);
    keep_with(&request, &log, &mut index, TRIES).expect("a run");
    assert_eq!(index.sorted(), rebuilt_with(&request, &log));
    assert!(index.rows.iter().filter(|row| row.first == 2).count() > 100);
}

#[test]
fn many_changed_entries_are_added_in_id_order_whatever_the_workers() {
    let grown = |workers: usize| {
        let mut log = Log::new();
        for n in 1..=60u8 {
            log.put(n, "lines", &["aaaaa", "bbbbb", "ccccc"]);
        }
        let mut index = Docs::default();
        keep(&log, &mut index);
        for n in (1..=60u8).rev() {
            log.append(n, &["ddddd", "eeeeeeeeeeeeeeeeeeeeeeee"]);
        }
        keep_with(&request_of(SMALL, workers), &log, &mut index, TRIES).expect("a run");
        assert_eq!(index.sorted(), rebuilt(&log));
        index.rows
    };
    let alone = grown(1);
    for workers in [0, 2, 8, 64] {
        assert_eq!(grown(workers), alone, "{workers} workers");
    }
}

#[test]
fn apply_refuses_a_tokenizer_the_plan_was_not_made_for() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    let mut index = Docs::default();
    let Plan::Work(work) = plan_of(&log, &mut index, TRIES) else {
        panic!("work");
    };
    let other = Tokenizer {
        tokens: Box::new(Bytes),
        id: "other/1",
    };
    let error =
        apply(work, &request(), &log, &log, &readers(), &other, &mut index).expect_err("a refusal");
    assert!(matches!(error, Error::Manifest), "{error:?}");
    assert_eq!((index.writes, index.commits), (0, 0));
    assert_eq!(index.position, None);
}

#[test]
fn a_rebuilds_work_names_every_entry_of_the_store_and_why() {
    let mut log = Log::new();
    for n in 1..=3 {
        log.put(n, "lines", &["aaaaa"]);
    }
    let mut index = Docs::default();
    let Plan::Work(work) = plan_of(&log, &mut index, TRIES) else {
        panic!("work");
    };
    assert_eq!((work.rebuilds(), work.entries()), (Some(Reason::Unmade), 3));
    keep(&log, &mut index);

    log.append(1, &["bbbbb"]);
    let Plan::Work(work) = plan_of(&log, &mut index, TRIES) else {
        panic!("work");
    };
    assert_eq!((work.rebuilds(), work.entries()), (None, 1));

    let wider = request_of(
        Size {
            tokens: 20,
            overlap: 2,
        },
        1,
    );
    let Plan::Work(work) = plan(&manifest_of(&wider), TRIES, &log, &mut index).expect("a plan")
    else {
        panic!("work");
    };
    assert_eq!(
        (work.rebuilds(), work.entries()),
        (Some(Reason::Manifest), 3)
    );

    let mut other = Log::new();
    other.catalog = [9; 16];
    for n in 1..=3 {
        other.put(n, "lines", &["aaaaa"]);
    }
    let Plan::Work(work) = plan_of(&other, &mut index, TRIES) else {
        panic!("work");
    };
    assert_eq!(
        (work.rebuilds(), work.entries()),
        (Some(Reason::Catalog), 3)
    );

    let mut ahead = Log::new();
    ahead.put(1, "lines", &[]);
    ahead.put(2, "lines", &[]);
    assert!(ahead.cursor().seq() < index.position.clone().expect("a commit").cursor.seq());
    let Plan::Work(work) = plan_of(&ahead, &mut index, TRIES) else {
        panic!("work");
    };
    assert_eq!(
        (work.rebuilds(), work.entries()),
        (Some(Reason::Catalog), 2)
    );
}

#[test]
fn apply_refuses_a_request_whose_chunk_size_or_rendering_the_plan_was_not_made_for() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa"]);
    let planned = request_of(
        Size {
            tokens: 512,
            overlap: 2,
        },
        1,
    );
    let others = [
        request_of(
            Size {
                tokens: 256,
                overlap: 2,
            },
            1,
        ),
        request_of(
            Size {
                tokens: 512,
                overlap: 3,
            },
            1,
        ),
        Request {
            render: "lines/2".to_owned(),
            ..planned.clone()
        },
    ];
    for other in others {
        let mut index = Docs::default();
        let Plan::Work(work) =
            plan(&manifest_of(&planned), TRIES, &log, &mut index).expect("a plan")
        else {
            panic!("work");
        };
        let error = apply(
            work,
            &other,
            &log,
            &log,
            &readers(),
            &tokenizer(),
            &mut index,
        )
        .expect_err("a refusal");
        assert!(matches!(error, Error::Manifest), "{error:?}");
        assert_eq!((index.writes, index.commits), (0, 0));
        assert_eq!(index.position, None);
        let again = plan(&manifest_of(&planned), TRIES, &log, &mut index).expect("a plan");
        let Plan::Work(work) = again else {
            panic!("work");
        };
        assert_eq!(work.rebuilds(), Some(Reason::Unmade));
    }
}

fn two_filed() -> (Log, Docs) {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "bbbbb"]);
    log.put(2, "lines", &["xxxxx", "yyyyy"]);
    log.label(1, "file:/home/me/src/garden");
    log.label(2, "file:/home/me/src/shed");
    let mut index = Docs::default();
    index.events = log.events.clone();
    keep(&log, &mut index);
    (log, index)
}

#[test]
fn labels_are_filed_before_the_index_commits() {
    let (mut log, mut index) = two_filed();
    log.events.borrow_mut().clear();
    log.append(1, &["ccccc"]);
    keep(&log, &mut index);
    let events = log.events.borrow();
    assert_eq!(events[events.len() - 2..], ["relabel", "commit"]);
    assert_eq!(
        events.iter().filter(|event| **event == "relabel").count(),
        1
    );
}

#[test]
fn a_failed_relabel_ends_the_run_before_the_commit() {
    let (mut log, mut index) = two_filed();
    log.events.borrow_mut().clear();
    log.append(1, &["ccccc"]);
    log.refuse_relabel.set(Some(log.relabels.borrow().len()));
    let error = keep_with(&request(), &log, &mut index, TRIES).expect_err("a refusal");
    assert!(matches!(error, Error::Store(_)), "{error:?}");
    assert!(!log.events.borrow().contains(&"commit"));
    assert_eq!(index.rolled_back, 1);
}

#[test]
fn an_update_stopped_before_it_files_anything_changes_no_label() {
    let (mut log, mut index) = two_filed();
    let (rows, position) = (index.sorted(), index.position.clone());
    let (calls, filed) = (log.relabels.borrow().len(), log.filed.borrow().clone());
    let adds = index.adds;
    log.append(1, &["ccccc"]);
    log.append(2, &["zzzzz"]);
    index.fail_entry = Some(id(2));
    let error = keep_with(&request(), &log, &mut index, TRIES).expect_err("a refusal");
    assert!(
        index.adds > adds,
        "entry 1 was added whole before the refusal"
    );
    assert!(matches!(error, Error::Index(_)), "{error:?}");
    assert_eq!(log.relabels.borrow().len(), calls);
    assert_eq!(*log.filed.borrow(), filed);
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    assert!(named(&index, &log));
}

#[test]
fn an_update_stopped_between_two_batches_leaves_every_entry_its_labels() {
    let mut log = Log::crowd(1100);
    let mut index = Docs::default();
    keep(&log, &mut index);
    let (rows, position) = (index.sorted(), index.position.clone());
    let base = log.relabels.borrow().len();
    log.append_all("more words");
    log.refuse_relabel.set(Some(base + 1));
    let error = keep_with(&request(), &log, &mut index, TRIES).expect_err("a refusal");
    assert!(matches!(error, Error::Store(_)), "{error:?}");
    let calls = log.relabels.borrow().clone();
    assert_eq!(calls.len() - base, 2);
    assert_eq!(calls[base].len(), 512);
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    assert_eq!(index.rolled_back, 1);
    assert!(named(&index, &log));
    log.refuse_relabel.set(None);
    keep(&log, &mut index);
    assert_eq!(index.sorted(), rebuilt(&log));
    assert!(named(&index, &log));
}

#[test]
fn an_update_whose_commit_is_refused_leaves_the_labels_filed_and_the_index_as_it_was() {
    let (mut log, mut index) = two_filed();
    let (rows, position) = (index.sorted(), index.position.clone());
    log.put(3, "lines", &["ccccc"]);
    log.label(3, "file:/home/me/src/greenhouse");
    log.append(1, &["ddddd"]);
    index.commit_fault = Fault::Refused;
    let error = keep_with(&request(), &log, &mut index, TRIES).expect_err("a refusal");
    assert!(matches!(error, Error::Index(_)), "{error:?}");
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    assert_eq!(index.rolled_back, 1);
    assert!(log.filed.borrow().contains_key(&id(3)));
    assert!(named(&index, &log));
}

#[test]
fn a_commit_that_publishes_and_then_reports_an_error_leaves_the_new_index_and_its_labels() {
    let (mut log, mut index) = two_filed();
    log.put(3, "lines", &["ccccc"]);
    log.label(3, "file:/home/me/src/greenhouse");
    log.append(1, &["ddddd"]);
    log.append(2, &["eeeee"]);
    index.commit_fault = Fault::AfterPublishing;
    let error = keep_with(&request(), &log, &mut index, TRIES).expect_err("an error");
    assert!(matches!(error, Error::Index(_)), "{error:?}");
    assert_eq!(index.rolled_back, 1);
    assert_eq!(index.sorted(), rebuilt(&log));
    assert_eq!(
        index
            .position
            .as_ref()
            .map(|position| position.cursor.clone()),
        Some(log.cursor())
    );
    assert!(log.filed.borrow().contains_key(&id(3)));
    assert!(named(&index, &log));
    index.commit_fault = Fault::None;
    assert_eq!(plan_of(&log, &mut index, TRIES), Plan::Current);
}

#[test]
fn a_rebuild_stopped_under_another_setting_leaves_the_first_index_current_and_its_labels_whole() {
    let log = Log::crowd(1100);
    let mut index = Docs::default();
    keep(&log, &mut index);
    let (rows, position) = (index.sorted(), index.position.clone());
    let other = request_of(
        Size {
            tokens: 16,
            overlap: 2,
        },
        1,
    );
    let base = log.relabels.borrow().len();
    log.refuse_relabel.set(Some(base + 1));
    let error = keep_with(&other, &log, &mut index, TRIES).expect_err("a refusal");
    assert!(matches!(error, Error::Store(_)), "{error:?}");
    assert_eq!(log.relabels.borrow().len() - base, 2);
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    assert_eq!(plan_of(&log, &mut index, TRIES), Plan::Current);
    assert!(named(&index, &log));
    for n in [1u16, 300, 513, 1100] {
        let word = label_words(&format!("place{n}"));
        assert_eq!(log.with_words(&word).expect("entries"), [wide(n)], "{n}");
    }
}

#[test]
fn a_session_that_grows_and_names_a_new_folder_is_filed_under_it_by_the_next_update() {
    let mut log = Log::new();
    log.put(1, "lines", &["aaaaa", "cwd:/home/me/src/garden"]);
    log.label(1, "file:/home/me/.claude/projects/garden/one.jsonl");
    let mut index = Docs::default();
    keep(&log, &mut index);
    assert!(
        log.with_words(&label_words("shed"))
            .expect("entries")
            .is_empty()
    );
    log.append(1, &["cwd:/home/me/src/shed"]);
    keep(&log, &mut index);
    let texts: Vec<String> = log.filed.borrow()[&id(1)]
        .iter()
        .map(|(text, _)| text.clone())
        .collect();
    assert_eq!(
        texts,
        [
            "file:/home/me/.claude/projects/garden/one.jsonl",
            "/home/me/src/garden",
            "/home/me/src/shed"
        ]
    );
    assert_eq!(index.sorted(), rebuilt(&log));
    assert!(named(&index, &log));
}
