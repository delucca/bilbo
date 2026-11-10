use std::fs;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use bilbo_store::entries::{Entries, StoreError};
use bilbo_store::layout::Layout;
use bilbo_store::limits::Compression;
use bilbo_store::segments::{Closed, Held, Hook, Notice, Options, Sealing, Step, Store};

use crate::scratch::{Scratch, keep, open_seg, origin, sealed, sealing};

const WAIT: Duration = Duration::from_secs(10);

fn flagging(hook: Option<Hook>) -> Options {
    Options {
        held: Held::Flag { readers: None },
        hook,
        ..sealing()
    }
}

fn again_file(layout: &Layout) -> std::path::PathBuf {
    layout.state.join("store.again")
}

#[test]
fn a_writer_that_finds_the_store_held_asks_the_holder_to_run_again_and_returns_at_once() {
    let scratch = Scratch::new("again_asked");
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
    let holder_layout = layout.clone();
    let holder = thread::spawn(move || {
        let mut store = Store::open(
            &holder_layout,
            Options {
                hook: Some(hook),
                ..sealing()
            },
            &mut |_| {},
        )
        .expect("the holder opens");
        keep(&mut store, "a.md", b"one\n");
        let Closed::Again(mut store) = store.close().expect("close") else {
            panic!("the holder must run again");
        };
        keep(&mut store, "b.md", b"two\n");
        matches!(store.close().expect("close again"), Closed::Released)
    });
    locked_rx.recv_timeout(WAIT).expect("the holder locks");

    let (done_tx, done_rx) = channel();
    let loser_layout = layout.clone();
    let loser = thread::spawn(move || {
        let result = Store::open(&loser_layout, flagging(None), &mut |_| {});
        done_tx.send(matches!(result, Err(StoreError::Busy))).ok();
    });
    assert_eq!(
        done_rx.recv_timeout(WAIT),
        Ok(true),
        "the loser fails Busy while the holder still holds the lock"
    );
    assert_eq!(fs::metadata(again_file(&layout)).expect("again").len(), 1);
    release_tx.send(()).expect("release");
    assert!(holder.join().expect("holder"), "one rerun, then released");
    loser.join().expect("loser");
    assert_eq!(fs::metadata(again_file(&layout)).expect("again").len(), 0);

    let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read");
    for name in ["a.md", "b.md"] {
        assert!(
            store.by_origin(&origin(name)).expect("head").is_some(),
            "{name}"
        );
    }
}

#[test]
fn a_request_made_while_the_holder_releases_is_never_lost() {
    let scratch = Scratch::new("again_release");
    let layout = scratch.layout();
    let (asking_tx, asking_rx) = channel::<()>();
    let (outcome_tx, outcome_rx) = channel::<bool>();
    let (free_tx, free_rx) = channel::<bool>();
    let loser_layout = layout.clone();
    let holder_layout = layout.clone();
    let hook: Hook = Box::new(move |step| {
        if step == Step::Released {
            let free = ["store.lock", "store.writer"].iter().all(|name| {
                fs::File::open(holder_layout.state.join(name))
                    .is_ok_and(|file| file.try_lock().is_ok())
            });
            free_tx.send(free).ok();
        }
        if step == Step::Releasing {
            let asking_tx = asking_tx.clone();
            let outcome_tx = outcome_tx.clone();
            let layout = loser_layout.clone();
            thread::spawn(move || {
                let asked: Hook = Box::new(move |step| {
                    if step == Step::Asking {
                        asking_tx.send(()).ok();
                    }
                    Ok(())
                });
                let result = Store::open(&layout, flagging(Some(asked)), &mut |_| {});
                let opened = result.is_ok();
                if let Ok(store) = result {
                    store.close().expect("the loser closes");
                }
                outcome_tx.send(opened).ok();
            });
            asking_rx
                .recv_timeout(WAIT)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    });
    let mut store = Store::open(
        &layout,
        Options {
            hook: Some(hook),
            ..sealing()
        },
        &mut |_| {},
    )
    .expect("open");
    keep(&mut store, "a.md", b"one\n");
    assert!(matches!(store.close().expect("close"), Closed::Released));
    assert_eq!(
        free_rx.recv_timeout(WAIT),
        Ok(true),
        "both store locks are free before the guard goes"
    );
    assert_eq!(
        outcome_rx.recv_timeout(WAIT),
        Ok(true),
        "a writer that found the store held as the holder released it gets the store itself"
    );
    assert_eq!(fs::metadata(again_file(&layout)).expect("again").len(), 0);
}

#[test]
fn several_requests_ask_for_one_more_run() {
    let scratch = Scratch::new("again_many");
    let layout = scratch.layout();
    let mut store = Store::open(&layout, sealing(), &mut |_| {}).expect("open");
    for _ in 0..3 {
        assert!(matches!(
            Store::open(&layout, flagging(None), &mut |_| {}),
            Err(StoreError::Busy)
        ));
    }
    keep(&mut store, "a.md", b"one\n");
    let Closed::Again(store) = store.close().expect("close") else {
        panic!("asked to run again");
    };
    assert!(matches!(store.close().expect("close"), Closed::Released));
}

#[test]
fn a_store_that_never_seals_keeps_its_frames_in_open_seg() {
    let scratch = Scratch::new("never_seals");
    let layout = scratch.layout();
    let mut store = Store::open(
        &layout,
        Options {
            seal_at: 1 << 20,
            compression: Compression::try_from(1).expect("a level"),
            sealing: Sealing::Never,
            ..sealing()
        },
        &mut |_| {},
    )
    .expect("open");
    let mut seed = 7u64;
    let big: Vec<u8> = (0..100_000)
        .flat_map(|_| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            format!("{seed:016x}{:016x}\n", seed.rotate_left(17)).into_bytes()
        })
        .collect();
    keep(&mut store, "a.md", &big);
    assert!(fs::metadata(open_seg(&layout)).expect("open.seg").len() > 1 << 20);
    assert!(matches!(store.close().expect("close"), Closed::Released));
    assert!(sealed(&layout).is_empty());

    let store = Store::open(&layout, sealing(), &mut |_| {}).expect("open");
    assert!(matches!(store.close().expect("close"), Closed::Released));
    assert_eq!(sealed(&layout).len(), 1);
}

#[test]
fn a_release_leaves_a_request_and_open_seg_for_the_next_close() {
    let scratch = Scratch::new("again_release");
    let layout = scratch.layout();
    let mut store = Store::open(
        &layout,
        Options {
            sealing: Sealing::Never,
            ..flagging(None)
        },
        &mut |_| {},
    )
    .expect("open");
    keep(&mut store, "a.md", b"one\n");
    fs::write(again_file(&layout), b"1").expect("a request from another writer");
    store.release();
    assert_eq!(
        fs::metadata(again_file(&layout)).expect("again").len(),
        1,
        "a release takes no request"
    );
    assert!(sealed(&layout).is_empty());
    assert!(fs::metadata(open_seg(&layout)).expect("open.seg").len() > 12);

    let store = Store::open(&layout, sealing(), &mut |_| {}).expect("the next writer");
    let Closed::Again(store) = store.close().expect("close") else {
        panic!("the next close finds the request");
    };
    assert!(matches!(
        store.close().expect("close again"),
        Closed::Released
    ));
    assert_eq!(fs::metadata(again_file(&layout)).expect("again").len(), 0);
    assert_eq!(sealed(&layout).len(), 1);
}

#[test]
fn a_writer_that_finds_only_readers_waits_for_them_and_asks_nobody_to_run_again() {
    let scratch = Scratch::new("again_readers");
    let layout = scratch.layout();
    let mut store = Store::open(&layout, sealing(), &mut |_| {}).expect("open");
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    let reader = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("a reader");

    let (waiting_tx, waiting_rx) = channel();
    let writer_layout = layout.clone();
    let writer = thread::spawn(move || {
        let mut store = Store::open(&writer_layout, flagging(None), &mut |notice| {
            if matches!(notice, Notice::Waiting) {
                waiting_tx.send(()).ok();
            }
        })
        .expect("the writer waits for the reader, then opens");
        keep(&mut store, "b.md", b"two\n");
        matches!(store.close().expect("close"), Closed::Released)
    });
    waiting_rx
        .recv_timeout(WAIT)
        .expect("the writer waits instead of failing Busy");
    assert_eq!(
        fs::metadata(again_file(&layout)).map_or(0, |meta| meta.len()),
        0,
        "no request is left for readers, which never run again"
    );
    drop(reader);
    assert!(writer.join().expect("writer"), "released without a rerun");

    let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read");
    assert!(store.by_origin(&origin("b.md")).expect("head").is_some());
}
