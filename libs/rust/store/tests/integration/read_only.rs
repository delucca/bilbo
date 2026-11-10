use std::fs;

use bilbo_store::entries::{CaptureError, Entries, Generation, Meta, RecordRange, StoreError};
use bilbo_store::layout::Layout;
use bilbo_store::segments::Store;

use crate::frames::spans;
use crate::scratch::{
    Scratch, catalog, id_of, keep, open, open_seg, origin, read_records, sealing, snapshot_of,
    try_keep,
};

/// `a.md` sealed, `b.md` left in open.seg by a writer that was dropped.
fn two_places(layout: &Layout) -> (bilbo_store::id::Id, bilbo_store::id::Id) {
    let mut store = open(layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    store.close().expect("close");
    let mut store = open(layout);
    let b = id_of(&keep(&mut store, "b.md", b"three\n"));
    drop(store);
    (a, b)
}

#[test]
fn a_read_only_store_reads_everything_and_writes_nothing() {
    let scratch = Scratch::new("read_only_reads");
    let layout = scratch.layout();
    let (a, b) = two_places(&layout);
    let frame = spans(&open_seg(&layout)).pop().expect("a frame");
    let whole = fs::read(open_seg(&layout)).expect("read");
    let mut torn = whole.clone();
    torn.extend_from_slice(&whole[frame.offset..frame.offset + 40]);
    fs::write(open_seg(&layout), &torn).expect("write");
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let before = snapshot_of(&layout);

    let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only open");
    assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n");
    assert_eq!(read_records(&store, b, 1, 1), b"three\n");
    assert_eq!(
        store.meta(b).expect("meta").map(|m| m.origin),
        Some(Some(origin("b.md")))
    );
    assert_eq!(
        store
            .by_origin(&origin("a.md"))
            .expect("head")
            .map(|h| h.id),
        Some(a)
    );
    drop(store);

    assert_eq!(snapshot_of(&layout), before);
    assert!(!catalog(&layout).exists());
    assert_eq!(
        fs::read(open_seg(&layout)).expect("read"),
        torn,
        "the torn tail is still there"
    );
}

#[test]
fn a_read_only_open_without_a_lock_file_or_a_state_folder_reads_the_sealed_segments_and_creates_nothing()
 {
    for case in ["no lock file", "no state folder"] {
        let scratch = Scratch::new("read_only_unlocked");
        let layout = scratch.layout();
        let (a, _) = two_places(&layout);
        fs::remove_file(catalog(&layout)).expect("delete the catalog");
        if case == "no lock file" {
            fs::remove_file(layout.state.join("store.lock")).expect("delete the lock file");
        } else {
            fs::remove_dir_all(&layout.state).expect("delete the state folder");
        }
        let before = snapshot_of(&layout);

        let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only open");
        assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n", "{case}");
        assert!(
            store.by_origin(&origin("b.md")).expect("head").is_none(),
            "{case}: open.seg is treated as absent"
        );
        drop(store);
        assert_eq!(snapshot_of(&layout), before, "{case}");
        assert_eq!(layout.state.exists(), case == "no lock file", "{case}");
    }
}

#[test]
fn a_read_only_store_refuses_writes() {
    let scratch = Scratch::new("read_only_refuses");
    let layout = scratch.layout();
    let (a, _) = two_places(&layout);
    let mut store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only open");
    let refused = try_keep(&mut store, "c.md", b"x\n", None);
    assert!(matches!(
        refused,
        Err(CaptureError::Store(StoreError::ReadOnly))
    ));
    let meta = Meta {
        parent: Some(a),
        ..store.meta(a).expect("meta").expect("some")
    };
    assert!(matches!(
        store.set_meta(a, &meta),
        Err(StoreError::ReadOnly)
    ));
    assert!(
        store
            .read(a, Generation(1), RecordRange { first: 1, count: 2 })
            .is_ok()
    );
}

#[test]
fn a_read_only_open_of_a_missing_store_says_so() {
    let scratch = Scratch::new("read_only_missing");
    let layout = scratch.layout();
    let error = Store::open_read_only(&layout, sealing(), &mut |_| {})
        .err()
        .expect("must fail");
    assert_eq!(
        error.to_string(),
        format!("no store at {}", layout.root.display())
    );
    assert!(!layout.root.exists());
}

#[test]
fn two_store_roots_cannot_share_one_state_folder() {
    let scratch = Scratch::new("shared_state");
    let first = scratch.layout();
    let mut store = open(&first);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");

    let second = Layout {
        root: first.root.with_file_name("root2"),
        state: first.state.clone(),
        cache: first.cache.with_file_name("cache2"),
    };
    fs::create_dir_all(&second.root).expect("root");
    fs::create_dir_all(&second.cache).expect("cache");
    let held = (snapshot_of(&first), snapshot_of(&second));
    let canonical_first = fs::canonicalize(&first.root).expect("canonical");
    let canonical_second = fs::canonicalize(&second.root).expect("canonical");
    let expected = bilbo_store::entries::StoreError::OtherRoot {
        state: first.state.clone(),
        recorded: canonical_first,
        asked: canonical_second,
    }
    .to_string();

    let writer = Store::open(&second, sealing(), &mut |_| {});
    assert_eq!(writer.err().map(|e| e.to_string()), Some(expected.clone()));
    let reader = Store::open_read_only(&second, sealing(), &mut |_| {});
    assert_eq!(reader.err().map(|e| e.to_string()), Some(expected));
    assert_eq!((snapshot_of(&first), snapshot_of(&second)), held);

    fs::remove_file(first.state.join("root")).expect("remove the record");
    assert!(Store::open_read_only(&second, sealing(), &mut |_| {}).is_ok());
}

/// `a.md` sealed and `b.md` in open.seg, the catalog on disk holding both.
fn two_places_cataloged(layout: &Layout) -> (bilbo_store::id::Id, bilbo_store::id::Id) {
    let mut store = open(layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    store.close().expect("close");
    let mut store = bilbo_store::segments::Store::open(
        layout,
        bilbo_store::segments::Options::default(),
        &mut |_| {},
    )
    .expect("open");
    let b = id_of(&keep(&mut store, "b.md", b"three\n"));
    store.close().expect("close without a seal");
    assert_eq!(crate::scratch::sealed(layout).len(), 1);
    (a, b)
}

#[test]
fn a_read_only_open_with_a_current_catalog_walks_no_segment_and_writes_nothing() {
    let scratch = Scratch::new("read_only_catalog");
    let layout = scratch.layout();
    let (a, b) = two_places_cataloged(&layout);
    // A sealed frame damaged after the catalog was written: a walk of the segments would refuse
    // the store, and only a read of that frame finds it.
    let segment = crate::scratch::sealed(&layout).pop().expect("a segment");
    let frame = spans(&segment).pop().expect("a frame");
    crate::scratch::flip(&segment, frame.offset + frame.total - 1);
    let before = snapshot_of(&layout);

    let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only open");
    assert_eq!(read_records(&store, b, 1, 1), b"three\n");
    assert_eq!(
        store
            .by_origin(&origin("a.md"))
            .expect("head")
            .map(|head| head.id),
        Some(a)
    );
    let damaged = store.read(a, Generation(1), RecordRange { first: 1, count: 2 });
    assert!(
        matches!(damaged, Err(StoreError::Damaged { .. })),
        "{damaged:?}"
    );
    drop(store);
    assert_eq!(snapshot_of(&layout), before, "no journal, no write");

    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let walked = Store::open_read_only(&layout, sealing(), &mut |_| {});
    assert!(matches!(walked, Err(StoreError::Damaged { .. })));
}

#[test]
fn a_read_only_open_walks_the_segments_when_the_catalog_cannot_be_used_as_it_is() {
    type Spoil = fn(&Layout);
    let cases: [(&str, Spoil); 6] = [
        ("the catalog is behind open.seg", |layout| {
            let whole = fs::read(open_seg(layout)).expect("read");
            let frame = spans(&open_seg(layout)).pop().expect("a frame");
            let mut torn = whole.clone();
            torn.extend_from_slice(&whole[frame.offset..frame.offset + 40]);
            fs::write(open_seg(layout), &torn).expect("write");
        }),
        ("the catalog is not a database", |layout| {
            fs::write(catalog(layout), b"not sqlite").expect("write");
        }),
        ("the catalog belongs to another store", |layout| {
            crate::scratch::sql(
                layout,
                "UPDATE catalog SET value = x'2f6f74686572' WHERE key = 'root'",
            );
        }),
        ("the catalog lists another segment", |layout| {
            crate::scratch::sql(
                layout,
                "UPDATE segments SET length = length + 1 WHERE name != 'open'",
            );
        }),
        ("a seal was interrupted", |layout| {
            fs::write(
                layout.state.join("sealing"),
                b"01JC5Q3V7XKM4P8R2T6W9YB0DE\n",
            )
            .expect("marker");
        }),
        ("the cache folder is open to others", |layout| {
            crate::scratch::chmod(&layout.cache, 0o777);
        }),
    ];
    for (case, spoil) in cases {
        let scratch = Scratch::new("read_only_fallback");
        let layout = scratch.layout();
        let (a, b) = two_places_cataloged(&layout);
        // The rows of the catalog on disk lead nowhere: only a walk of the segments reads `a.md`.
        crate::scratch::sql(&layout, "UPDATE frames SET offset = offset + 1");
        spoil(&layout);
        let before = snapshot_of(&layout);
        let store = Store::open_read_only(&layout, sealing(), &mut |_| {})
            .unwrap_or_else(|error| panic!("{case}: {error}"));
        assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n", "{case}");
        assert_eq!(read_records(&store, b, 1, 1), b"three\n", "{case}");
        drop(store);
        assert_eq!(snapshot_of(&layout), before, "{case}");
        crate::scratch::chmod(&layout.cache, 0o700);
    }
}
