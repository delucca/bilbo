use std::sync::mpsc::{TryRecvError, channel};
use std::thread;
use std::time::Duration;

use bilbo_store::entries::{Entries, Generation, RecordRange};
use bilbo_store::segments::{Hook, Notice, Options, Step, Store};

use crate::scratch::{Scratch, id_of, keep, open, read_records};

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn a_second_writer_waits_for_the_first_and_says_so() {
    let scratch = Scratch::new("second_writer");
    let layout = scratch.layout();
    let (locked_tx, locked_rx) = channel::<()>();
    let (release_tx, release_rx) = channel::<()>();
    let hook: Hook = Box::new(move |step| {
        if step == Step::Locked {
            locked_tx.send(()).ok();
            release_rx
                .recv_timeout(WAIT)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    });
    let first_layout = layout.clone();
    let first = thread::spawn(move || {
        let mut store = Store::open(
            &first_layout,
            Options {
                hook: Some(hook),
                ..Options::default()
            },
            &mut |_| {},
        )
        .expect("first open");
        keep(&mut store, "a.md", b"one\n");
        store.close().expect("first close");
    });
    locked_rx
        .recv_timeout(WAIT)
        .expect("the first writer holds the lock");

    let (waiting_tx, waiting_rx) = channel::<Notice>();
    let (opened_tx, opened_rx) = channel::<bool>();
    let second_layout = layout.clone();
    let second = thread::spawn(move || {
        let store = Store::open(&second_layout, Options::default(), &mut |notice| {
            waiting_tx.send(notice).ok();
        })
        .expect("second open");
        let found = store
            .by_origin(&crate::scratch::origin("a.md"))
            .expect("head")
            .is_some();
        opened_tx.send(found).ok();
    });
    assert_eq!(waiting_rx.recv_timeout(WAIT), Ok(Notice::Waiting));
    assert_eq!(opened_rx.try_recv(), Err(TryRecvError::Empty));
    release_tx.send(()).expect("release");
    assert_eq!(
        opened_rx.recv_timeout(WAIT),
        Ok(true),
        "the second open returns after the first closes, and sees its entry"
    );
    first.join().expect("first thread");
    second.join().expect("second thread");
}

#[test]
fn a_reader_waits_for_a_writer_and_sees_its_seal_whole() {
    let scratch = Scratch::new("reader_waits");
    let layout = scratch.layout();
    let (locked_tx, locked_rx) = channel::<()>();
    let (release_tx, release_rx) = channel::<()>();
    let hook: Hook = Box::new(move |step| {
        if step == Step::Locked {
            locked_tx.send(()).ok();
            release_rx
                .recv_timeout(WAIT)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    });
    let writer_layout = layout.clone();
    let writer = thread::spawn(move || {
        let mut store = Store::open(
            &writer_layout,
            Options {
                hook: Some(hook),
                ..Options::default()
            },
            &mut |_| {},
        )
        .expect("writer open");
        keep(&mut store, "a.md", b"one\ntwo\n");
        keep(&mut store, "b.md", b"three\n");
        store.close().expect("writer close");
    });
    locked_rx
        .recv_timeout(WAIT)
        .expect("the writer holds the lock");

    let (waiting_tx, waiting_rx) = channel::<Notice>();
    let (read_tx, read_rx) = channel::<Vec<Vec<u8>>>();
    let reader_layout = layout.clone();
    let reader = thread::spawn(move || {
        let store = Store::open_read_only(&reader_layout, Options::default(), &mut |notice| {
            waiting_tx.send(notice).ok();
        })
        .expect("read-only open");
        let mut found = Vec::new();
        for name in ["a.md", "b.md"] {
            let head = store
                .by_origin(&crate::scratch::origin(name))
                .expect("head")
                .expect("entry");
            found.push(
                store
                    .read(
                        head.id,
                        Generation(1),
                        RecordRange {
                            first: 1,
                            count: head.records,
                        },
                    )
                    .expect("read")
                    .concat(),
            );
        }
        read_tx.send(found).ok();
    });
    assert_eq!(waiting_rx.recv_timeout(WAIT), Ok(Notice::Waiting));
    assert_eq!(read_rx.try_recv().err(), Some(TryRecvError::Empty));
    release_tx.send(()).expect("release");
    assert_eq!(
        read_rx.recv_timeout(WAIT).expect("the reader's bytes"),
        vec![b"one\ntwo\n".to_vec(), b"three\n".to_vec()]
    );
    writer.join().expect("writer thread");
    reader.join().expect("reader thread");
}

#[test]
fn a_writer_waits_for_a_reader_and_the_readers_reads_stay_valid() {
    let scratch = Scratch::new("writer_waits");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    let b = id_of(&keep(&mut store, "b.md", b"three\n"));
    drop(store);

    let reader =
        Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("read-only open");
    let (waiting_tx, waiting_rx) = channel::<Notice>();
    let (opened_tx, opened_rx) = channel::<()>();
    let writer_layout = layout.clone();
    let writer = thread::spawn(move || {
        let store = Store::open(&writer_layout, Options::default(), &mut |notice| {
            waiting_tx.send(notice).ok();
        })
        .expect("writer open");
        opened_tx.send(()).ok();
        store.close().expect("writer close");
    });
    assert_eq!(waiting_rx.recv_timeout(WAIT), Ok(Notice::Waiting));
    assert_eq!(read_records(&reader, a, 1, 2), b"one\ntwo\n");
    assert_eq!(read_records(&reader, b, 1, 1), b"three\n");
    assert_eq!(opened_rx.try_recv(), Err(TryRecvError::Empty));
    drop(reader);
    opened_rx
        .recv_timeout(WAIT)
        .expect("the writer opens once the reader is gone");
    writer.join().expect("writer thread");

    let again =
        Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("read-only open");
    assert_eq!(read_records(&again, a, 1, 2), b"one\ntwo\n");
    assert_eq!(read_records(&again, b, 1, 1), b"three\n");
}
