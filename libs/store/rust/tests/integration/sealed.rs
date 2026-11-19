use std::time::SystemTime;

use bilbo_store::id::Id;
use bilbo_store::layout::Layout;
use bilbo_store::sealed::Sealed;
use bilbo_store::segments::{Closed, Store};

use crate::scratch::{Scratch, id_of, keep, open, open_with, sealed, sealing, sql};

fn closed(store: Store) {
    assert!(matches!(store.close().expect("close"), Closed::Released));
}

fn named(layout: &Layout, index: usize) -> SystemTime {
    let path = &sealed(layout)[index];
    let stem = path.file_stem().expect("stem").to_string_lossy();
    Id::parse(&stem).expect("an id").minted()
}

#[test]
fn an_entry_whose_newest_records_are_in_open_seg_is_not_sealed() {
    let scratch = Scratch::new("sealed_open");
    let layout = scratch.layout();
    let mut store = open_with(&layout, bilbo_store::segments::Options::default());
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    assert_eq!(store.sealed_at(id).expect("answers"), None);
    closed(store);
    assert!(sealed(&layout).is_empty());
}

#[test]
fn a_sealed_entry_answers_the_time_in_its_segments_name() {
    let scratch = Scratch::new("sealed_named");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    let before = SystemTime::now();
    closed(store);
    let after = SystemTime::now();
    assert_eq!(sealed(&layout).len(), 1);
    let store = open(&layout);
    let at = store.sealed_at(id).expect("answers").expect("sealed");
    assert_eq!(at, named(&layout, 0));
    let millis = |at: SystemTime| {
        at.duration_since(SystemTime::UNIX_EPOCH)
            .expect("after 1970")
            .as_millis()
    };
    assert!((millis(before)..=millis(after)).contains(&millis(at)));
}

#[test]
fn an_entry_that_grew_after_a_seal_is_not_sealed_until_the_next_seal() {
    let scratch = Scratch::new("sealed_grew");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    closed(store);
    let first = sealed(&layout)[0].clone();
    let mut store = open_with(&layout, bilbo_store::segments::Options::default());
    keep(&mut store, "a.md", b"one\ntwo\n");
    assert_eq!(store.sealed_at(id).expect("answers"), None);
    closed(store);
    let mut store = open(&layout);
    assert_eq!(store.sealed_at(id).expect("answers"), None);
    keep(&mut store, "b.md", b"x\n");
    closed(store);
    let store = open(&layout);
    let second = store.sealed_at(id).expect("answers").expect("sealed");
    assert_eq!(sealed(&layout).len(), 2);
    assert_eq!(second, named(&layout, 1));
    let made = sealed(&layout)
        .into_iter()
        .find(|name| *name != first)
        .expect("the second seal's segment");
    assert!(made.file_name() > first.file_name());
}

#[test]
fn the_entry_asked_is_the_one_answered() {
    let scratch = Scratch::new("sealed_two_entries");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let first = id_of(&keep(&mut store, "a.md", b"one\n"));
    closed(store);
    let mut store = open_with(&layout, bilbo_store::segments::Options::default());
    let second = id_of(&keep(&mut store, "b.md", b"one\n"));
    keep(&mut store, "b.md", b"one\ntwo\n");
    assert_eq!(sealed(&layout).len(), 1);
    assert_eq!(
        store.sealed_at(first).expect("answers"),
        Some(named(&layout, 0))
    );
    assert_eq!(store.sealed_at(second).expect("answers"), None);
}

#[test]
fn an_entry_captured_empty_and_sealed_then_grown_is_not_sealed() {
    let scratch = Scratch::new("sealed_empty_then_grown");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b""));
    closed(store);
    let mut store = open_with(&layout, bilbo_store::segments::Options::default());
    assert_eq!(
        store.sealed_at(id).expect("answers"),
        Some(named(&layout, 0))
    );
    keep(&mut store, "a.md", b"one\n");
    assert_eq!(store.sealed_at(id).expect("answers"), None);
}

#[test]
fn only_the_newest_generation_is_asked() {
    let scratch = Scratch::new("sealed_generation");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    closed(store);
    let mut store = open_with(&layout, bilbo_store::segments::Options::default());
    keep(&mut store, "a.md", b"one\nTWO\nthree\n");
    assert_eq!(store.sealed_at(id).expect("answers"), None);
    closed(store);
    let mut store = open(&layout);
    assert_eq!(store.sealed_at(id).expect("answers"), None);
    keep(&mut store, "a.md", b"");
    assert_eq!(store.sealed_at(id).expect("answers"), None);
    closed(store);
    let store = open(&layout);
    assert_eq!(
        store.sealed_at(id).expect("answers"),
        Some(named(&layout, 1))
    );
    assert_eq!(sealed(&layout).len(), 2);
}

#[test]
fn an_entry_the_store_does_not_hold_is_not_sealed() {
    let scratch = Scratch::new("sealed_absent");
    let store = open(&scratch.layout());
    let id = Id::mint().expect("mint");
    assert_eq!(store.sealed_at(id).expect("no error"), None);
}

#[test]
fn a_store_opened_to_read_answers_as_the_writer_did() {
    let scratch = Scratch::new("sealed_read_only");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    closed(store);
    let writer = open(&layout);
    let written = writer.sealed_at(id).expect("answers");
    closed(writer);
    let reader = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only open");
    assert_eq!(reader.sealed_at(id).expect("answers"), written);
    assert!(written.is_some());
}

#[test]
fn a_newest_generation_with_no_frame_is_not_sealed() {
    let scratch = Scratch::new("sealed_no_frame");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    closed(store);
    let store = open(&layout);
    assert_eq!(
        store.sealed_at(id).expect("answers"),
        Some(named(&layout, 0))
    );
    closed(store);
    sql(
        &layout,
        "INSERT INTO generations SELECT entry, 2, records, origin_end, origin_hash FROM generations",
    );
    let store = open(&layout);
    assert_eq!(store.sealed_at(id).expect("answers"), None);
    assert_eq!(sealed(&layout).len(), 1);
}

#[test]
fn a_sealed_segment_whose_name_is_no_id_is_sealed_at_the_epoch() {
    let scratch = Scratch::new("sealed_no_id");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    closed(store);
    let store = open(&layout);
    assert_eq!(
        store.sealed_at(id).expect("answers"),
        Some(named(&layout, 0))
    );
    closed(store);
    sql(&layout, "UPDATE frames SET segment = 'x'");
    let store = open(&layout);
    assert_eq!(
        store.sealed_at(id).expect("answers"),
        Some(SystemTime::UNIX_EPOCH)
    );
}
