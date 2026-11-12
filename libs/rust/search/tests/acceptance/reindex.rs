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

use crate::fakes::{
    Bytes, CATALOG, Call, Kept, Lines, Log, Marked, Recorded, Watched, at, id, kept, shelf, unread,
};

const SMALL: Size = Size {
    tokens: 12,
    overlap: 2,
};

fn request() -> Request {
    Request {
        chunk: SMALL,
        render: "lines/1".to_owned(),
        workers: 1,
    }
}

fn tokenizer(tokens: impl Tokens + 'static) -> Tokenizer {
    Tokenizer {
        tokens: Box::new(tokens),
        id: "bytes/1",
    }
}

fn readers() -> Vec<Box<dyn Reader>> {
    vec![Box::new(Lines("lines")), Box::new(Lines("notes"))]
}

fn reindex(shelf: &Log, index: &mut Recorded) -> Result<Output, Error> {
    run(&request(), shelf, &readers(), &tokenizer(Bytes), index)
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
        let output = run(
            &request,
            &shelf(entries()),
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
    let error = run(
        &request,
        &shelf(entries),
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
        let ran = run(
            &request,
            &shelf(entries()),
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
        run(
            &request,
            &shelf(entries),
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
    let output = run(
        &request,
        &shelf(vec![kept(1, "lines", &["first\n", big, "last\n"])]),
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
