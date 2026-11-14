use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use bilbo_kinds::reader::{Passage, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_search::analyze;
use bilbo_search::chunk::Size;
use bilbo_search::position::{Manifest, Position};
use bilbo_search::reindex::run;
use bilbo_search::tokenizer::Tokenizer;
use bilbo_search::work::{Error, Output, Request};

use bilbo_store::entries::Cursor;
use bilbo_store::id::Id;
use bilbo_store::labels::Labels;

use crate::fakes::{
    Bytes, CATALOG, Call, Docs, Fault, Foldered, Kept, Lines, Log, Marked, Recorded, Watched, at,
    id, kept, named, shelf, unread, wide,
};
use bilbo_search::analyze::label_words;

const SMALL: Size = Size {
    tokens: 12,
    overlap: 2,
};

fn request() -> Request {
    Request {
        chunk: SMALL,
        render: "lines/1".to_owned(),
        workers: 1,
        label_batch: 512,
    }
}

fn tokenizer(tokens: impl Tokens + 'static) -> Tokenizer {
    Tokenizer {
        tokens: Box::new(tokens),
        id: "bytes/1",
    }
}

fn readers() -> Vec<Box<dyn Reader>> {
    vec![Box::new(Foldered("lines")), Box::new(Lines("notes"))]
}

fn reindex(shelf: &Log, index: &mut Recorded) -> Result<Output, Error> {
    run(
        &request(),
        shelf,
        shelf,
        &readers(),
        &tokenizer(Bytes),
        index,
    )
}

fn position(seq: u64) -> Position {
    Position {
        manifest: Manifest::new(SMALL, "bytes/1", "lines/1"),
        cursor: Cursor::at(CATALOG, seq),
        failed: Vec::new(),
    }
}

#[test]
fn every_entry_is_indexed_in_id_order_and_committed_once_with_the_stores_cursor() {
    let shelf = shelf(vec![
        kept(3, "lines", &["cccc\n"]),
        kept(1, "lines", &["aaaa\n", "bbbb\n", "dddddddd\n"]),
        kept(2, "notes", &["eeee\n"]),
    ]);
    let mut index = Recorded {
        bytes: 4096,
        ..Recorded::default()
    };
    let output = reindex(&shelf, &mut index).expect("a run");
    assert_eq!(
        output,
        Output {
            entries: 3,
            chunks: 4,
            index_bytes: 4096,
            ..Output::default()
        }
    );
    let calls = index.calls();
    assert_eq!(calls[0], Call::Clear);
    assert_eq!(calls[calls.len() - 1], Call::Commit(position(6)));
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::Commit(_) | Call::Clear))
            .count(),
        2
    );
    assert_eq!(
        index.added(),
        [
            (id(1), "aaaa\nbbbb".to_owned()),
            (id(1), "dddddddd".to_owned()),
            (id(2), "eeee".to_owned()),
            (id(3), "cccc".to_owned()),
        ]
    );
    assert_eq!(
        calls[1],
        Call::Add {
            entry: id(1),
            generation: 1,
            kind: "lines".to_owned(),
            date: at(1001),
            first: 1,
            count: 2,
            part: None,
            text: "aaaa\nbbbb".to_owned(),
        }
    );
}

#[test]
fn only_the_newest_generation_of_an_entry_is_indexed() {
    let mut rewritten = kept(1, "lines", &["old one\n", "old two\n"]);
    rewritten.generations.push(vec!["new\n"]);
    let mut index = Recorded::default();
    let output = reindex(&shelf(vec![rewritten]), &mut index).expect("a run");
    assert_eq!((output.entries, output.chunks), (1, 1));
    assert_eq!(
        index.calls()[1],
        Call::Add {
            entry: id(1),
            generation: 2,
            kind: "lines".to_owned(),
            date: at(1001),
            first: 1,
            count: 1,
            part: None,
            text: "new".to_owned(),
        }
    );
    assert_eq!(index.calls()[2], Call::Commit(position(3)));
}

type Window = (u64, Option<(u32, u32)>, String);

#[test]
fn a_record_over_the_size_is_indexed_as_its_windows() {
    let mut index = Recorded::default();
    let long = kept(1, "lines", &["short\n", "abcdefghijklmnopqrst\n"]);
    reindex(&shelf(vec![long]), &mut index).expect("a run");
    let parts: Vec<Window> = index
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Add {
                first, part, text, ..
            } => Some((first, part, text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        parts,
        [
            (1, None, "short".to_owned()),
            (2, Some((0, 5)), "abcdef".to_owned()),
            (2, Some((1, 5)), "line:\nefghij".to_owned()),
            (2, Some((2, 5)), "line:\nijklmn".to_owned()),
            (2, Some((3, 5)), "line:\nmnopqr".to_owned()),
            (2, Some((4, 5)), "line:\nqrst".to_owned()),
        ]
    );
}

#[test]
fn an_entry_takes_the_time_of_its_first_dated_passage_or_else_when_it_was_captured() {
    let mut index = Recorded::default();
    let entries = vec![
        kept(
            1,
            "lines",
            &["-\n", "undated\n", "@50 first\n", "@70 later\n"],
        ),
        kept(2, "lines", &["undated\n"]),
    ];
    reindex(&shelf(entries), &mut index).expect("a run");
    let dates: Vec<_> = index
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Add { entry, date, .. } => Some((entry, date)),
            _ => None,
        })
        .collect();
    assert_eq!(dates, [(id(1), at(50)), (id(1), at(50)), (id(2), at(1002))]);
}

#[test]
fn an_entry_of_a_kind_no_reader_reads_is_left_out_and_counted_by_kind() {
    let mut index = Recorded::default();
    let entries = vec![
        kept(1, "pdf", &["%PDF\n"]),
        kept(2, "lines", &["kept\n"]),
        kept(3, "pdf", &["%PDF\n"]),
        kept(4, "audio", &["cue\n"]),
    ];
    let output = reindex(&shelf(entries), &mut index).expect("a run");
    assert_eq!((output.entries, output.chunks), (1, 1));
    assert_eq!(output.unread, unread(&[("audio", 1), ("pdf", 2)]));
    assert!(output.failed.is_empty());
    assert_eq!(index.added(), [(id(2), "kept".to_owned())]);
}

#[test]
fn an_entry_that_renders_nothing_or_holds_no_record_is_an_entry_with_no_chunk() {
    let mut index = Recorded::default();
    let entries = vec![
        kept(1, "lines", &["-\n", "-\n"]),
        kept(2, "lines", &[]),
        kept(3, "lines", &["text\n"]),
    ];
    let output = reindex(&shelf(entries), &mut index).expect("a run");
    assert_eq!((output.entries, output.chunks), (3, 1));
    assert_eq!(index.added(), [(id(3), "text".to_owned())]);
}

#[test]
fn an_entry_the_store_cannot_read_back_is_reported_and_the_rest_is_indexed_and_committed() {
    let mut index = Recorded::default();
    let mut damaged = kept(2, "lines", &["lost\n"]);
    damaged.damaged = true;
    let entries = vec![
        kept(1, "lines", &["one\n"]),
        damaged,
        kept(3, "lines", &["three\n"]),
    ];
    let output = reindex(&shelf(entries), &mut index).expect("a run");
    assert_eq!((output.entries, output.chunks), (2, 2));
    assert_eq!(
        output.failed,
        [(
            id(2),
            "/s/a.seg is damaged at byte 12: checksum mismatch".to_owned()
        )]
    );
    assert_eq!(
        index.added(),
        [(id(1), "one".to_owned()), (id(3), "three".to_owned())]
    );
    assert_eq!(committed(&index).failed, [(id(2), 1)]);
}

fn committed(index: &Recorded) -> Position {
    match index.calls().pop() {
        Some(Call::Commit(position)) => position,
        other => panic!("a commit, not {other:?}"),
    }
}

#[test]
fn a_run_with_no_failed_entry_commits_an_empty_list() {
    let mut index = Recorded::default();
    reindex(&shelf(vec![kept(1, "lines", &["one\n"])]), &mut index).expect("a run");
    assert_eq!(committed(&index).failed, []);
}

#[test]
fn an_empty_store_gives_an_empty_committed_index() {
    let mut index = Recorded::default();
    let output = reindex(&shelf(Vec::new()), &mut index).expect("a run");
    assert_eq!(output, Output::default());
    assert_eq!(index.calls(), [Call::Clear, Call::Commit(position(0))]);
}

#[test]
fn a_failing_index_stops_the_run_and_nothing_is_committed() {
    let entries = || vec![kept(1, "lines", &["one\n"]), kept(2, "lines", &["two\n"])];
    for (fail_at, calls) in [(0, 0), (1, 1), (2, 2), (3, 3)] {
        let mut index = Recorded {
            fail_at: Some(fail_at),
            ..Recorded::default()
        };
        let error = reindex(&shelf(entries()), &mut index).expect_err("a failure");
        assert!(matches!(error, Error::Index(_)), "{fail_at}: {error:?}");
        assert_eq!(
            error.to_string(),
            "cannot write the index /c/index: disk full"
        );
        assert_eq!(index.calls().len(), calls);
        assert!(
            !index
                .calls()
                .iter()
                .any(|call| matches!(call, Call::Commit(_)))
        );
    }
}

#[test]
fn a_store_that_cannot_list_its_changes_stops_the_run_before_the_index_is_touched() {
    let mut index = Recorded::default();
    let mut broken = shelf(vec![kept(1, "lines", &["one\n"])]);
    broken.changes_fail = true;
    let error = reindex(&broken, &mut index).expect_err("a failure");
    assert!(matches!(error, Error::Store(_)), "{error:?}");
    assert!(index.calls().is_empty());
}

#[test]
fn a_manifest_that_cannot_be_written_down_stops_the_run_before_the_index_is_touched() {
    let mut index = Recorded::default();
    let mut request = request();
    request.render = "two\nlines".to_owned();
    let entries = shelf(vec![kept(1, "lines", &["one\n"])]);
    let error = run(
        &request,
        &entries,
        &entries,
        &readers(),
        &tokenizer(Bytes),
        &mut index,
    )
    .expect_err("a failure");
    assert!(matches!(error, Error::Manifest), "{error:?}");
    assert!(index.calls().is_empty());
}

#[test]
fn the_index_gets_the_same_calls_in_the_same_order_however_many_workers_chunk() {
    let texts: Vec<&'static str> = vec![
        "one\n",
        "abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz\n",
        "-\n",
        "three three\n",
    ];
    let entries = || -> Vec<Kept> {
        (1..=60)
            .map(|n| {
                let records: Vec<&'static str> = texts
                    .iter()
                    .cycle()
                    .skip(n as usize % 4)
                    .take(n as usize % 7)
                    .copied()
                    .collect();
                kept(n, if n % 9 == 0 { "pdf" } else { "lines" }, &records)
            })
            .collect()
    };
    let calls = |workers: usize| {
        let mut index = Recorded::default();
        let request = Request {
            workers,
            ..request()
        };
        let log = shelf(entries());
        let output = run(
            &request,
            &log,
            &log,
            &readers(),
            &tokenizer(Bytes),
            &mut index,
        );
        (output.expect("a run"), index.calls())
    };
    let alone = calls(1);
    assert!(alone.1.len() > 100);
    for workers in [0, 2, 3, 8, 64] {
        assert_eq!(calls(workers), alone, "{workers} workers");
    }
}

#[test]
fn a_failing_index_stops_a_run_of_several_workers_too() {
    let entries: Vec<Kept> = (1..=40)
        .map(|n| kept(n, "lines", &["some text\n"]))
        .collect();
    let mut index = Recorded {
        fail_at: Some(7),
        ..Recorded::default()
    };
    let request = Request {
        workers: 4,
        ..request()
    };
    let log = shelf(entries);
    let error = run(
        &request,
        &log,
        &log,
        &readers(),
        &tokenizer(Bytes),
        &mut index,
    )
    .expect_err("a failure");
    assert!(matches!(error, Error::Index(_)), "{error:?}");
    assert_eq!(index.calls().len(), 7);
}

struct Ran {
    output: Result<Output, String>,
    added: Vec<(Id, String)>,
    calls: Vec<Call>,
}

/// Runs on a thread of its own and fails by timing out instead of hanging the suite.
fn run_marked(workers: usize, entries: fn() -> Vec<Kept>) -> Ran {
    let (sent, received) = mpsc::channel();
    thread::spawn(move || {
        let mut index = Recorded::default();
        let request = Request {
            workers,
            ..request()
        };
        let log = shelf(entries());
        let ran = run(
            &request,
            &log,
            &log,
            &readers(),
            &tokenizer(Marked),
            &mut index,
        );
        let _ = sent.send(Ran {
            output: ran.map_err(|error| error.to_string()),
            added: index.added(),
            calls: index.calls(),
        });
    });
    received
        .recv_timeout(Duration::from_secs(20))
        .expect("the run ends")
}

fn twelve(first: &'static str) -> Vec<Kept> {
    let mut entries = vec![kept(1, "lines", &[first])];
    entries.extend((2..=12).map(|n| kept(n, "lines", &["some text\n"])));
    entries
}

fn committed_by(calls: &[Call]) -> Position {
    match calls.last() {
        Some(Call::Commit(position)) => position.clone(),
        other => panic!("a commit, not {other:?}"),
    }
}

fn panicked_and_the_rest_is_indexed(workers: usize, entries: fn() -> Vec<Kept>) {
    let Ran {
        output,
        added,
        calls,
    } = run_marked(workers, entries);
    let output = output.expect("a run");
    assert_eq!(
        committed_by(&calls).failed,
        [(id(1), 1)],
        "{workers} workers"
    );
    assert_eq!(
        output.failed,
        [(id(1), "its reader or the tokenizer panicked".to_owned())],
        "{workers} workers"
    );
    assert_eq!(
        (output.entries, output.chunks),
        (11, 11),
        "{workers} workers"
    );
    let wanted: Vec<(Id, String)> = (2..=12).map(|n| (id(n), "some text".to_owned())).collect();
    assert_eq!(added, wanted, "{workers} workers");
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::Commit(_)))
            .count(),
        1
    );
}

#[test]
fn an_entry_whose_reader_panics_is_reported_and_the_rest_is_indexed() {
    for workers in [1, 2, 8] {
        panicked_and_the_rest_is_indexed(workers, || twelve("!panic\n"));
    }
}

#[test]
fn an_entry_whose_panic_payload_panics_when_dropped_is_reported_and_the_rest_is_indexed() {
    for workers in [1, 2] {
        panicked_and_the_rest_is_indexed(workers, || twelve("!bomb\n"));
    }
}

#[test]
fn an_entry_whose_tokenizer_panics_is_reported_and_the_rest_is_indexed() {
    for workers in [1, 2, 8] {
        panicked_and_the_rest_is_indexed(workers, || twelve("a PANIC here\n"));
    }
}

#[test]
fn an_entry_the_tokenizer_cannot_split_is_reported_and_none_of_its_chunks_is_indexed() {
    for workers in [1, 3] {
        let Ran {
            output,
            added,
            calls,
        } = run_marked(workers, || {
            vec![
                kept(1, "lines", &["one\n"]),
                kept(2, "lines", &["fine\n", "has UNSPLIT in it\n"]),
                kept(3, "lines", &["three\n"]),
            ]
        });
        let output = output.expect("a run");
        assert_eq!(
            output.failed,
            [(
                id(2),
                "the tokenizer could not split a text of it".to_owned()
            )]
        );
        assert_eq!((output.entries, output.chunks), (2, 2));
        assert_eq!(committed_by(&calls).failed, [(id(2), 1)]);
        assert_eq!(
            added,
            [(id(1), "one".to_owned()), (id(3), "three".to_owned())]
        );
    }
}

#[test]
fn the_manifest_of_a_run_names_the_analyzer_the_tokenizer_and_the_rendering() {
    let mut index = Recorded::default();
    reindex(&shelf(vec![kept(1, "lines", &["one\n"])]), &mut index).expect("a run");
    let Some(Call::Commit(committed)) = index.calls().pop() else {
        panic!("a commit");
    };
    assert_eq!(committed.manifest.analyzer, analyze::ID);
    assert_eq!(committed.manifest.tokenizer, "bytes/1");
    assert_eq!(committed.manifest.render, "lines/1");
    assert_eq!(committed.manifest.chunk, SMALL);
    assert_eq!(
        committed.manifest,
        Manifest::new(SMALL, "bytes/1", "lines/1")
    );
}

struct Gated {
    inner: Lines,
    gone: Arc<(Mutex<bool>, Condvar)>,
}

impl Reader for Gated {
    fn kind(&self) -> &'static str {
        "gated"
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        let (flag, signal) = &*self.gone;
        let has = |word: &[u8]| records.iter().any(|record| record.starts_with(word));
        if has(b"hold") {
            let mut gone = flag.lock().expect("the flag");
            while !*gone {
                let (next, waited) = signal
                    .wait_timeout(gone, Duration::from_secs(10))
                    .expect("the flag");
                gone = next;
                assert!(
                    !waited.timed_out(),
                    "the entry that opens the gate was never read"
                );
            }
        }
        let passages = self.inner.passages(records);
        if has(b"go") {
            *flag.lock().expect("the flag") = true;
            signal.notify_all();
        }
        passages
    }

    fn appends_only(&self) -> bool {
        true
    }
}

#[test]
fn an_entry_made_first_waits_for_the_one_before_it() {
    for workers in [2, 8] {
        let gone = Arc::new((Mutex::new(false), Condvar::new()));
        let readers: Vec<Box<dyn Reader>> = vec![
            Box::new(Lines("lines")),
            Box::new(Gated {
                inner: Lines("gated"),
                gone,
            }),
        ];
        let mut entries = vec![
            kept(1, "gated", &["hold first\n"]),
            kept(2, "gated", &["go second\n"]),
        ];
        entries.extend((3..=8).map(|n| kept(n, "lines", &["ordinary\n"])));
        let mut index = Recorded::default();
        let request = Request {
            workers,
            ..request()
        };
        let log = shelf(entries);
        run(
            &request,
            &log,
            &log,
            &readers,
            &tokenizer(Bytes),
            &mut index,
        )
        .expect("a run");
        let ids: Vec<Id> = index.added().into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ids,
            (1..=8).map(id).collect::<Vec<_>>(),
            "{workers} workers"
        );
    }
}

#[test]
fn an_entry_with_a_record_over_a_megabyte_is_indexed_whole_and_the_tokenizer_never_sees_it_whole() {
    let lines: Vec<String> = (0..)
        .map(|n| format!("some short line {n:07} of a long record"))
        .take(45_000)
        .collect();
    let big: &'static str = Box::leak(
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>()
            .into_boxed_str(),
    );
    assert!(big.len() > 1 << 20);
    let longest = Arc::new(AtomicUsize::default());
    let mut index = Recorded::default();
    let request = Request {
        chunk: Size {
            tokens: 4096,
            overlap: 64,
        },
        ..request()
    };
    let log = shelf(vec![kept(1, "lines", &["first\n", big, "last\n"])]);
    let output = run(
        &request,
        &log,
        &log,
        &readers(),
        &tokenizer(Watched(Arc::clone(&longest))),
        &mut index,
    )
    .expect("a run");
    assert!(output.failed.is_empty());
    let adds: Vec<(u64, String)> = index
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Add { first, text, .. } => Some((first, text)),
            _ => None,
        })
        .collect();
    let firsts: Vec<u64> = adds.iter().map(|(first, _)| *first).collect();
    let wanted: HashSet<&str> = lines.iter().map(String::as_str).collect();
    let whole: HashSet<&str> = adds
        .iter()
        .flat_map(|(_, text)| text.split('\n'))
        .filter(|line| wanted.contains(line))
        .collect();
    assert_eq!(whole.len(), lines.len());
    assert_eq!(firsts.first(), Some(&1));
    assert_eq!(firsts.last(), Some(&3));
    assert!(firsts.iter().filter(|first| **first == 2).count() > 100);
    let longest = longest.load(Ordering::Relaxed);
    assert!(longest <= (1 << 20) + "line:\n".len(), "{longest}");
}

fn filed_text(log: &Log, entry: Id) -> Option<Vec<String>> {
    log.filed
        .borrow()
        .get(&entry)
        .map(|rows| rows.iter().map(|(text, _)| text.clone()).collect())
}

fn whole(log: &Log, index: &mut Docs) -> Result<Output, Error> {
    run(&request(), log, log, &readers(), &tokenizer(Bytes), index)
}

fn shared(log: &Log) -> Docs {
    let mut index = Docs::default();
    index.events = log.events.clone();
    index
}

#[test]
fn labels_are_filed_before_the_index_commits() {
    let mut log = shelf(vec![
        kept(1, "lines", &["aaaa\n"]),
        kept(2, "lines", &["bbbb\n"]),
        kept(3, "lines", &["cccc\n"]),
    ]);
    log.label(1, "file:/home/me/src/garden");
    let mut index = shared(&log);
    whole(&log, &mut index).expect("a run");
    let events = log.events.borrow();
    assert_eq!(events[events.len() - 2..], ["relabel", "commit"]);
    assert_eq!(
        events.iter().filter(|event| **event == "relabel").count(),
        1
    );
    assert_eq!(events.iter().filter(|event| **event == "commit").count(), 1);
}

#[test]
fn an_entry_the_run_reads_is_filed_under_its_capture_label() {
    let mut log = shelf(vec![
        kept(1, "lines", &["aaaa\n"]),
        kept(2, "lines", &["bbbb\n"]),
    ]);
    log.label(1, "file:/home/me/src/Garden");
    log.filed.borrow_mut().insert(
        id(2),
        vec![("file:/stale".to_owned(), label_words("file:/stale"))],
    );
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    assert_eq!(
        log.filed.borrow().get(&id(1)),
        Some(&vec![(
            "file:/home/me/src/Garden".to_owned(),
            label_words("file:/home/me/src/Garden")
        )])
    );
    assert_eq!(label_words("file:/home/me/src/Garden").len(), 5);
    assert_eq!(log.filed.borrow().get(&id(2)), None);
    let calls = log.relabels.borrow();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .iter()
            .any(|filed| filed.entry == id(2) && filed.labels.is_empty())
    );
}

#[test]
fn an_entry_no_reader_reads_is_in_no_call() {
    let mut log = shelf(vec![
        kept(1, "lines", &["aaaa\n"]),
        kept(2, "strange", &["bbbb\n"]),
    ]);
    log.label(2, "file:/home/me/src/old");
    let rows = vec![("file:/home/me/src/old".to_owned(), vec!["old".to_owned()])];
    log.filed.borrow_mut().insert(id(2), rows.clone());
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    assert!(
        log.relabels
            .borrow()
            .iter()
            .flatten()
            .all(|filed| filed.entry != id(2))
    );
    assert_eq!(log.filed.borrow().get(&id(2)), Some(&rows));
}

#[test]
fn an_entry_whose_reader_fails_keeps_the_rows_it_had() {
    let mut log = shelf(vec![
        kept(1, "lines", &["fine\n"]),
        kept(2, "lines", &["!panic\n"]),
        kept(3, "lines", &["an UNSPLIT text\n"]),
        kept(4, "lines", &["damaged\n"]),
        kept(5, "lines", &["!panic\n"]),
    ]);
    for n in 1..=5 {
        log.label(n, &format!("file:/home/me/src/place{n}"));
    }
    log.retouch(4, |held| held.damaged = true);
    for n in [3, 4] {
        let text = format!("file:/before/{n}");
        log.filed
            .borrow_mut()
            .insert(id(n), vec![(text.clone(), label_words(&text))]);
    }
    let mut index = Docs::default();
    run(
        &request(),
        &log,
        &log,
        &readers(),
        &tokenizer(Marked),
        &mut index,
    )
    .expect("a run");
    let calls = log.relabels.borrow();
    let in_calls: Vec<Id> = calls.iter().flatten().map(|filed| filed.entry).collect();
    assert_eq!(in_calls, [id(1)]);
    assert_eq!(filed_text(&log, id(2)), None);
    assert_eq!(filed_text(&log, id(5)), None);
    assert_eq!(
        filed_text(&log, id(3)),
        Some(vec!["file:/before/3".to_owned()])
    );
    assert_eq!(
        filed_text(&log, id(4)),
        Some(vec!["file:/before/4".to_owned()])
    );

    let mut log = shelf(vec![kept(
        1,
        "lines",
        &["cwd:/home/me/src/garden\n", "cwd:/home/me/src/shed\n"],
    )]);
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    let both = Some(vec![
        "/home/me/src/garden".to_owned(),
        "/home/me/src/shed".to_owned(),
    ]);
    assert_eq!(filed_text(&log, id(1)), both);
    log.append(1, &["!panic"]);
    log.relabels.borrow_mut().clear();
    let output = whole(&log, &mut index).expect("a run");
    assert_eq!(output.failed.len(), 1);
    assert!(
        log.relabels
            .borrow()
            .iter()
            .flatten()
            .all(|filed| filed.entry != id(1))
    );
    assert_eq!(filed_text(&log, id(1)), both);
}

#[test]
fn what_a_reader_finds_is_filed_after_the_capture_label() {
    let mut log = shelf(vec![
        kept(
            1,
            "lines",
            &[
                "cwd:/home/me/src/shed\n",
                "cwd:file:/home/me/src/garden\n",
                "cwd:/home/me/src/shed\n",
                "cwd:/home/me/Notes\n",
            ],
        ),
        kept(2, "lines", &["cwd:/home/me/src/shed\n"]),
    ]);
    log.label(1, "file:/home/me/src/garden");
    whole(&log, &mut Docs::default()).expect("a run");
    let texts = [
        "file:/home/me/src/garden",
        "/home/me/src/shed",
        "/home/me/Notes",
    ];
    assert_eq!(
        log.filed.borrow().get(&id(1)),
        Some(
            &texts
                .iter()
                .map(|text| ((*text).to_owned(), label_words(text)))
                .collect::<Vec<_>>()
        )
    );
    assert_eq!(
        log.filed.borrow().get(&id(2)),
        Some(&vec![(
            "/home/me/src/shed".to_owned(),
            label_words("/home/me/src/shed")
        )])
    );
    assert_eq!(label_words("/home/me/Notes"), ["home", "me", "note"]);
}

#[test]
fn a_reader_that_panics_while_it_gives_labels_fails_the_entry() {
    let mut log = shelf(vec![
        kept(1, "lines", &["cwd:/home/me/src/garden\n"]),
        kept(2, "lines", &["fine\n", "!labels-panic\n"]),
        kept(3, "lines", &["cwd:/home/me/src/shed\n"]),
    ]);
    log.label(2, "file:/home/me/src/orchard");
    let mut index = Docs::default();
    let output = whole(&log, &mut index).expect("a run");
    assert_eq!(output.failed.len(), 1);
    assert_eq!(output.failed[0].0, id(2));
    assert!(index.sorted().iter().all(|row| row.entry != id(2)));
    assert!(index.sorted().iter().any(|row| row.entry == id(1)));
    assert!(index.sorted().iter().any(|row| row.entry == id(3)));
    let in_calls: Vec<Id> = log
        .relabels
        .borrow()
        .iter()
        .flatten()
        .map(|filed| filed.entry)
        .collect();
    assert_eq!(in_calls, [id(1), id(3)]);
    assert_eq!(filed_text(&log, id(2)), None);
}

#[test]
fn an_entry_the_store_no_longer_has_loses_its_labels_when_the_fill_has_ended() {
    let mut log = shelf(vec![
        kept(1, "lines", &["aaaa\n"]),
        kept(2, "lines", &["bbbb\n"]),
        kept(3, "lines", &["cccc\n"]),
    ]);
    for n in 1..=3 {
        log.label(n, &format!("file:/home/me/src/place{n}"));
    }
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    log.remove(2);
    log.relabels.borrow_mut().clear();
    let mut again = Docs::default();
    whole(&log, &mut again).expect("a run");
    let calls = log.relabels.borrow();
    assert_eq!(calls.len(), 1);
    let order: Vec<(Id, bool)> = calls[0]
        .iter()
        .map(|filed| (filed.entry, filed.labels.is_empty()))
        .collect();
    assert_eq!(order, [(id(1), false), (id(3), false), (id(2), true)]);
    assert_eq!(filed_text(&log, id(2)), None);
}

fn call_sizes(log: &Log) -> Vec<usize> {
    log.relabels.borrow().iter().map(Vec::len).collect()
}

#[test]
fn a_run_of_many_entries_files_them_in_id_order_a_batch_at_a_time() {
    let log = Log::crowd(1100);
    whole(&log, &mut Docs::default()).expect("a run");
    assert_eq!(call_sizes(&log), [512, 512, 76]);
    let order: Vec<Id> = log
        .relabels
        .borrow()
        .iter()
        .flatten()
        .map(|filed| filed.entry)
        .collect();
    assert_eq!(order, (1..=1100).map(wide).collect::<Vec<_>>());
    let log = Log::crowd(1024);
    whole(&log, &mut Docs::default()).expect("a run");
    assert_eq!(call_sizes(&log), [512, 512]);
}

#[test]
fn the_label_batch_bounds_the_entries_filed_at_once() {
    let log = Log::crowd(5);
    let request = Request {
        label_batch: 2,
        ..request()
    };
    let mut index = Docs::default();
    run(
        &request,
        &log,
        &log,
        &readers(),
        &tokenizer(Bytes),
        &mut index,
    )
    .expect("a run");
    assert_eq!(call_sizes(&log), [2, 2, 1]);
    assert!(named(&index, &log));
    for n in 1..=5u16 {
        let word = label_words(&format!("place{n}"));
        assert_eq!(log.with_words(&word).expect("entries"), [wide(n)], "{n}");
    }
}

#[test]
fn a_rebuild_files_as_an_update_does_and_empties_nothing() {
    let mut log = Log::crowd(600);
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    let before = log.filed.borrow().clone();
    log.relabels.borrow_mut().clear();
    log.relabel_all(|entry| format!("file:/new/{}", entry.to_bytes()[15]));
    whole(&log, &mut index).expect("a rebuild");
    let calls = log.relabels.borrow();
    assert_eq!(calls.iter().map(Vec::len).collect::<Vec<_>>(), [512, 88]);
    assert!(calls.iter().flatten().all(|filed| !filed.labels.is_empty()));
    assert_eq!(log.filed.borrow().len(), before.len());
}

#[test]
fn a_rebuild_stopped_after_it_emptied_the_index_changes_no_label() {
    let mut log = shelf(vec![
        kept(1, "lines", &["aaaa\n"]),
        kept(2, "lines", &["bbbb\n"]),
    ]);
    log.label(1, "file:/home/me/src/garden");
    log.label(2, "file:/home/me/src/shed");
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    let (rows, position, filed) = (
        index.sorted(),
        index.position.clone(),
        log.filed.borrow().clone(),
    );
    let calls = log.relabels.borrow().len();
    log.label(1, "file:/home/me/src/greenhouse");
    index.fail_add = Some(index.adds);
    let error = whole(&log, &mut index).expect_err("a refusal");
    assert!(matches!(error, Error::Index(_)), "{error:?}");
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    assert_eq!(log.relabels.borrow().len(), calls);
    assert_eq!(*log.filed.borrow(), filed);
    assert!(named(&index, &log));
    index.fail_add = None;
    whole(&log, &mut index).expect("a run");
    assert_eq!(
        filed_text(&log, id(1)),
        Some(vec!["file:/home/me/src/greenhouse".to_owned()])
    );
    assert!(named(&index, &log));
}

#[test]
fn a_rebuild_stopped_between_two_batches_leaves_every_entry_its_labels() {
    let mut log = Log::crowd(1100);
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    let (rows, position) = (index.sorted(), index.position.clone());
    let base = log.relabels.borrow().len();
    log.relabel_all(|entry| {
        format!(
            "file:/new/{}",
            u16::from_be_bytes([entry.to_bytes()[14], entry.to_bytes()[15]])
        )
    });
    log.append_all("cwd:/home/me/src/new");
    log.refuse_relabel.set(Some(base + 1));
    let error = whole(&log, &mut index).expect_err("a refusal");
    assert!(matches!(error, Error::Store(_)), "{error:?}");
    let calls = log.relabels.borrow().clone();
    assert_eq!(calls.len() - base, 2);
    let first: Vec<Id> = calls[base].iter().map(|filed| filed.entry).collect();
    assert_eq!(first, (1..=512).map(wide).collect::<Vec<_>>());
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    assert_eq!(index.rolled_back, 1);
    assert_eq!(
        filed_text(&log, wide(1)),
        Some(vec![
            "file:/new/1".to_owned(),
            "/home/me/src/new".to_owned()
        ])
    );
    assert_eq!(
        filed_text(&log, wide(513)),
        Some(vec!["file:/home/me/src/place513".to_owned()])
    );
    assert!(named(&index, &log));
    log.refuse_relabel.set(None);
    whole(&log, &mut index).expect("a run");
    assert_eq!(
        filed_text(&log, wide(1100)),
        Some(vec![
            "file:/new/1100".to_owned(),
            "/home/me/src/new".to_owned()
        ])
    );
    assert!(named(&index, &log));
}

#[test]
fn a_rebuild_whose_commit_is_refused_leaves_the_labels_filed_and_the_index_as_it_was() {
    let mut log = shelf(vec![
        kept(1, "lines", &["aaaa\n"]),
        kept(2, "lines", &["bbbb\n"]),
    ]);
    log.label(1, "file:/home/me/src/garden");
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    let (rows, position) = (index.sorted(), index.position.clone());
    log.put(3, "lines", &["cccc\n"]);
    log.label(3, "file:/home/me/src/shed");
    index.commit_fault = Fault::Refused;
    let error = whole(&log, &mut index).expect_err("a refusal");
    assert!(matches!(error, Error::Index(_)), "{error:?}");
    assert_eq!((index.sorted(), index.position.clone()), (rows, position));
    assert_eq!(index.rolled_back, 1);
    assert_eq!(
        filed_text(&log, id(3)),
        Some(vec!["file:/home/me/src/shed".to_owned()])
    );
    assert!(named(&index, &log));
}

#[test]
fn a_failed_relabel_ends_the_run_before_the_commit() {
    let log = shelf(vec![kept(1, "lines", &["aaaa\n"])]);
    log.refuse_relabel.set(Some(0));
    let mut index = shared(&log);
    let error = whole(&log, &mut index).expect_err("a refusal");
    assert!(matches!(error, Error::Store(_)), "{error:?}");
    assert!(!log.events.borrow().contains(&"commit"));
    assert_eq!((index.commits, index.rolled_back), (0, 1));
}

#[test]
fn the_check_of_a_stopped_run_refuses_older_labels_and_wrong_words() {
    let mut log = shelf(vec![kept(
        1,
        "lines",
        &["body\n", "cwd:/home/me/src/shed\n"],
    )]);
    log.label(1, "file:/home/me/src/garden");
    let mut index = Docs::default();
    whole(&log, &mut index).expect("a run");
    assert!(named(&index, &log));
    let row = |text: &str| (text.to_owned(), label_words(text));
    let garden = "file:/home/me/src/garden";
    log.filed.borrow_mut().insert(id(1), vec![row(garden)]);
    assert!(!named(&index, &log), "the capture label alone");
    log.filed.borrow_mut().insert(
        id(1),
        [garden, "/home/me/src/shed"]
            .map(|text| (text.to_owned(), Vec::new()))
            .to_vec(),
    );
    assert!(!named(&index, &log), "words emptied");
    let shed = "/home/me/src/shed";
    log.filed.borrow_mut().insert(id(1), vec![row(shed)]);
    assert!(!named(&index, &log), "no capture label");
    log.filed
        .borrow_mut()
        .insert(id(1), [shed, garden].map(row).to_vec());
    assert!(named(&index, &log), "the labels in another order");
    log.append(1, &["cwd:/home/me/src/greenhouse\n"]);
    log.filed.borrow_mut().insert(
        id(1),
        [garden, "/home/me/src/shed", "/home/me/src/greenhouse"]
            .map(row)
            .to_vec(),
    );
    assert!(named(&index, &log), "a newer prefix");
    log.rewrite(1, &["other\n", "cwd:/home/me/src/greenhouse\n"]);
    whole(&log, &mut index).expect("a run");
    assert!(named(&index, &log), "the second generation");
    log.filed
        .borrow_mut()
        .insert(id(1), [garden, shed].map(row).to_vec());
    assert!(!named(&index, &log), "the first generation's labels");
}

#[test]
fn a_label_of_more_words_than_the_bound_is_filed_under_its_first_ones() {
    let text = (0..70)
        .map(|n| format!("w{n}"))
        .collect::<Vec<_>>()
        .join("/");
    let mut log = Log::new();
    log.put(1, "lines", &[&format!("cwd:{text}")]);
    whole(&log, &mut Docs::default()).expect("a run");
    let all = label_words(&text);
    assert_eq!(all.len(), 70);
    assert_eq!(
        log.filed.borrow().get(&id(1)),
        Some(&vec![(text, all[..analyze::MAX_LABEL_WORDS].to_vec())])
    );
    assert_eq!(
        log.with_words(&["w63".to_owned()]).expect("entries"),
        [id(1)]
    );
    assert!(
        log.with_words(&["w64".to_owned()])
            .expect("entries")
            .is_empty()
    );
}
