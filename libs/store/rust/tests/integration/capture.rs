use std::fs;

use bilbo_store::entries::{
    Capture, CaptureError, Captured, Change, Cursor, Entries, Generation, Meta, RecordRange,
    StoreError,
};
use bilbo_store::id::Id;
use bilbo_store::labels::StoredLabels;

use crate::scratch::{
    Scratch, capture_label, catalog, id_of, keep, keep_as, open, open_seg, read_records, try_keep,
};

#[test]
fn a_first_capture_is_new_and_reads_back_byte_for_byte() {
    let scratch = Scratch::new("first_capture");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let bytes = b"one\ntwo\nthree";
    let captured = keep(&mut store, "a.md", bytes);
    let Captured::New(new) = captured else {
        panic!("expected New, got {captured:?}");
    };
    assert_eq!(new.generation, Generation(1));
    assert_eq!(new.records, RecordRange { first: 1, count: 3 });
    assert_eq!(new.raw_bytes, bytes.len() as u64);
    assert_eq!(
        new.stored_bytes,
        fs::metadata(open_seg(&layout)).expect("open.seg").len() - 12
    );
    assert_eq!(read_records(&store, new.id, 1, 3), bytes);
    let middle = store
        .read(new.id, Generation(1), RecordRange { first: 2, count: 1 })
        .expect("read");
    assert_eq!(middle, vec![b"two\n".to_vec()]);
    let meta = store.meta(new.id).expect("meta").expect("some");
    assert_eq!(meta.kind, "markdown");
    assert_eq!(meta.labels.capture, Some(capture_label("a.md")));
    assert_eq!(meta.labels.id, None);
    let head = store
        .by_capture(&capture_label("a.md"))
        .expect("head")
        .expect("some");
    assert_eq!(head.id, new.id);
    assert_eq!(head.generation, Generation(1));
    assert_eq!(head.records, 3);
    assert_eq!(head.mark.end, bytes.len() as u64);
}

#[test]
fn the_same_bytes_again_are_unchanged_and_write_nothing() {
    let scratch = Scratch::new("unchanged");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let first = keep(&mut store, "a.md", b"one\ntwo\n");
    let before = fs::metadata(open_seg(&layout)).expect("open.seg").len();
    let again = keep(&mut store, "a.md", b"one\ntwo\n");
    assert_eq!(again, Captured::Unchanged(id_of(&first)));
    assert_eq!(
        fs::metadata(open_seg(&layout)).expect("open.seg").len(),
        before
    );
}

#[test]
fn bytes_after_the_prefix_are_appended_as_the_next_records() {
    let scratch = Scratch::new("append");
    let mut store = open(&scratch.layout());
    let id = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    let grown = b"one\ntwo\nthree\nfour";
    let Captured::Appended(appended) = keep(&mut store, "a.md", grown) else {
        panic!("expected Appended");
    };
    assert_eq!(appended.id, id);
    assert_eq!(appended.generation, Generation(1));
    assert_eq!(appended.records, RecordRange { first: 3, count: 2 });
    assert_eq!(appended.raw_bytes, "three\nfour".len() as u64);
    assert_eq!(read_records(&store, id, 1, 4), grown);
}

#[test]
fn a_changed_prefix_starts_a_new_generation_and_the_old_one_stays_readable() {
    let scratch = Scratch::new("new_generation");
    let mut store = open(&scratch.layout());
    let id = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    let Captured::NewGeneration(second) = keep(&mut store, "a.md", b"one\nTWO\nthree\n") else {
        panic!("expected NewGeneration");
    };
    assert_eq!(second.id, id);
    assert_eq!(second.generation, Generation(2));
    assert_eq!(second.records, RecordRange { first: 1, count: 3 });
    assert_eq!(second.raw_bytes, "one\nTWO\nthree\n".len() as u64);
    assert_eq!(read_records(&store, id, 1, 2), b"one\ntwo\n");
    assert_eq!(read_records(&store, id, 2, 3), b"one\nTWO\nthree\n");
    let head = store
        .by_capture(&capture_label("a.md"))
        .expect("head")
        .expect("some");
    assert_eq!(head.generation, Generation(2));
}

#[test]
fn an_empty_file_stays_unchanged_grows_by_append_and_empties_into_a_new_generation() {
    let scratch = Scratch::new("empty_file");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let first = keep(&mut store, "e.md", b"");
    let Captured::New(new) = first else {
        panic!("expected New");
    };
    let id = new.id;
    assert_eq!(new.records, RecordRange { first: 1, count: 0 });
    drop(store);

    let mut store = open(&layout);
    assert_eq!(keep(&mut store, "e.md", b""), Captured::Unchanged(id));
    let Captured::Appended(grown) = keep(&mut store, "e.md", b"x\n") else {
        panic!("expected Appended");
    };
    assert_eq!(grown.generation, Generation(1));
    assert_eq!(grown.records, RecordRange { first: 1, count: 1 });
    let Captured::NewGeneration(emptied) = keep(&mut store, "e.md", b"") else {
        panic!("expected NewGeneration");
    };
    assert_eq!(emptied.generation, Generation(2));
    assert_eq!(emptied.records, RecordRange { first: 1, count: 0 });
    drop(store);

    let mut store = open(&layout);
    assert_eq!(keep(&mut store, "e.md", b""), Captured::Unchanged(id));
    drop(store);

    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let mut store = open(&layout);
    assert_eq!(keep(&mut store, "e.md", b""), Captured::Unchanged(id));
    assert_eq!(read_records(&store, id, 1, 1), b"x\n");
    assert!(
        store
            .read(id, Generation(2), RecordRange { first: 1, count: 1 })
            .is_err()
    );
    assert_eq!(
        store
            .read(id, Generation(2), RecordRange { first: 1, count: 0 })
            .expect("read"),
        Vec::<Vec<u8>>::new()
    );
}

#[test]
fn a_given_id_is_used_at_first_capture_only() {
    let scratch = Scratch::new("given_id");
    let mut store = open(&scratch.layout());
    let given = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
    let other = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("id");
    let first = keep_as(&mut store, "a.md", b"one\n", Some(given));
    assert_eq!(id_of(&first), given);
    let grown = keep_as(&mut store, "a.md", b"one\ntwo\n", Some(other));
    assert_eq!(id_of(&grown), given);
    assert!(store.meta(other).expect("meta").is_none());
}

#[test]
fn an_id_held_by_another_capture_label_is_refused() {
    let scratch = Scratch::new("id_clash");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let given = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
    keep_as(&mut store, "a.md", b"one\n", Some(given));
    let before = fs::metadata(open_seg(&layout)).expect("open.seg").len();
    let refused = try_keep(&mut store, "b.md", b"two\n", Some(given));
    match refused {
        Err(CaptureError::IdHeld { id, by }) => {
            assert_eq!(id, given);
            assert_eq!(by, Some(capture_label("a.md")));
        }
        other => panic!("expected IdHeld, got {other:?}"),
    }
    assert_eq!(
        fs::metadata(open_seg(&layout)).expect("open.seg").len(),
        before
    );
    assert!(
        store
            .by_capture(&capture_label("b.md"))
            .expect("head")
            .is_none()
    );
}

#[test]
fn records_that_do_not_cover_the_bytes_are_refused() {
    let scratch = Scratch::new("bad_records");
    let mut store = open(&scratch.layout());
    let label = capture_label("a.md");
    let refused = store.capture(&Capture {
        label: &label,
        kind: "markdown",
        id: None,
        id_label: None,
        stamp: None,
        bytes: b"one\ntwo\n",
        ends: &[4],
    });
    assert!(matches!(refused, Err(CaptureError::Records(_))));
}

#[test]
fn set_meta_returns_the_length_of_the_meta_frame_it_wrote() {
    let scratch = Scratch::new("set_meta_grew");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    let meta = store.meta(id).expect("meta").expect("some");
    let before = fs::metadata(open_seg(&layout)).expect("open.seg").len();
    let grew = store.set_meta(id, &meta).expect("set_meta");
    let after = fs::metadata(open_seg(&layout)).expect("open.seg").len();
    assert!(grew > 0);
    assert_eq!(grew, after - before);
}

#[test]
fn set_meta_replaces_the_meta_and_survives_a_rebuild() {
    let scratch = Scratch::new("set_meta");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    let mut meta = store.meta(id).expect("meta").expect("some");
    meta.kind = "other".into();
    meta.labels.id = Some(Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("id"));
    meta.labels.capture = Some(capture_label("moved.md"));
    store.set_meta(id, &meta).expect("set_meta");
    assert_eq!(store.meta(id).expect("meta"), Some(meta.clone()));
    assert!(
        store
            .by_capture(&capture_label("a.md"))
            .expect("head")
            .is_none()
    );
    assert_eq!(
        store
            .by_capture(&capture_label("moved.md"))
            .expect("head")
            .map(|h| h.id),
        Some(id)
    );

    let wrong = Meta {
        id: Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("id"),
        ..meta.clone()
    };
    assert!(matches!(
        store.set_meta(id, &wrong),
        Err(StoreError::MetaMismatch { .. })
    ));
    assert!(matches!(
        store.set_meta(wrong.id, &wrong),
        Err(StoreError::Missing(_))
    ));
    let other = id_of(&keep(&mut store, "b.md", b"two\n"));
    let clash = Meta {
        labels: StoredLabels {
            capture: Some(capture_label("moved.md")),
            id: None,
        },
        ..store.meta(other).expect("meta").expect("some")
    };
    assert_eq!(
        store.set_meta(other, &clash).map_err(|e| e.to_string()),
        Err(format!(
            "the capture label file:/notes/moved.md is held by {id}"
        ))
    );
    drop(store);

    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let store = open(&layout);
    assert_eq!(store.meta(id).expect("meta"), Some(meta));
}

#[test]
fn changes_list_every_meta_and_body_frame_in_order() {
    let scratch = Scratch::new("changes");
    let mut store = open(&scratch.layout());
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    let b = id_of(&keep(&mut store, "b.md", b"x\ny\n"));
    keep(&mut store, "a.md", b"one\ntwo\n");
    let meta = store.meta(a).expect("meta").expect("some");
    store.set_meta(a, &meta).expect("set_meta");
    keep(&mut store, "b.md", b"changed\n");
    let (changes, cursor) = store.changes(&Cursor::start()).expect("changes");
    let records = |id, generation, first, count| Change::Records {
        id,
        generation: Generation(generation),
        records: RecordRange { first, count },
    };
    assert_eq!(
        changes,
        vec![
            Change::Meta(a),
            records(a, 1, 1, 1),
            Change::Meta(b),
            records(b, 1, 1, 2),
            records(a, 1, 2, 1),
            Change::Meta(a),
            records(b, 2, 1, 1),
        ]
    );
    assert!(
        changes
            .iter()
            .all(|change| change.id() == a || change.id() == b)
    );
    assert_eq!(store.changes(&cursor).expect("changes").0, Vec::new());
    keep(&mut store, "c.md", b"z\n");
    let (later, _) = store.changes(&cursor).expect("changes");
    assert_eq!(later.len(), 2);
}

#[test]
fn a_cursor_from_another_catalog_starts_over() {
    let scratch = Scratch::new("foreign_cursor");
    let mut store = open(&scratch.layout());
    keep(&mut store, "a.md", b"one\n");
    keep(&mut store, "b.md", b"two\n");
    let (all, _) = store.changes(&Cursor::start()).expect("changes");
    assert_eq!(all.len(), 4);
    let (again, _) = store.changes(&Cursor::at([9; 16], 3)).expect("changes");
    assert_eq!(again, all);
}

#[test]
fn an_id_label_is_kept_in_the_meta_and_survives_a_rebuild() {
    let scratch = Scratch::new("id_label");
    let layout = scratch.layout();
    let id_label = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("id");
    let mut store = open(&layout);
    let label = capture_label("agent-a.jsonl");
    let captured = store
        .capture(&Capture {
            label: &label,
            kind: "session",
            id: None,
            id_label: Some(id_label),
            stamp: None,
            bytes: b"{}\n",
            ends: &[3],
        })
        .expect("capture");
    let id = id_of(&captured);
    assert_eq!(
        store.meta(id).expect("meta").map(|m| m.labels.id),
        Some(Some(id_label))
    );
    let plain = id_of(&keep(&mut store, "a.md", b"one\n"));
    assert_eq!(
        store.meta(plain).expect("meta").map(|m| m.labels.id),
        Some(None)
    );
    store.close().expect("close");

    fs::remove_file(catalog(&layout)).expect("drop the catalog");
    let store = open(&layout);
    assert_eq!(
        store.meta(id).expect("meta").map(|m| m.labels.id),
        Some(Some(id_label))
    );
    assert_eq!(
        store.meta(plain).expect("meta").map(|m| m.labels.id),
        Some(None)
    );
}

#[test]
fn a_capture_without_a_stamp_clears_the_old_one_so_a_restore_is_a_new_generation() {
    let scratch = Scratch::new("stamp-clear");
    let layout = scratch.layout();
    let settled = bilbo_store::entries::Stamp {
        size: 2,
        modified: std::time::UNIX_EPOCH
            + std::time::Duration::from_nanos(1_700_000_000_000_000_000),
    };
    let mut store = open(&layout);
    let label = capture_label("a.md");
    let capture = |store: &mut bilbo_store::segments::Store,
                   bytes: &[u8],
                   stamp: Option<bilbo_store::entries::Stamp>| {
        store
            .capture(&Capture {
                label: &label,
                kind: "markdown",
                id: None,
                id_label: None,
                stamp,
                bytes,
                ends: &crate::scratch::ends(bytes),
            })
            .expect("capture")
    };
    let head = |store: &bilbo_store::segments::Store| {
        store.by_capture(&label).expect("head").expect("an entry")
    };
    capture(&mut store, b"a\n", Some(settled));
    assert_eq!(head(&store).stamp, Some(settled));

    assert!(matches!(
        capture(&mut store, b"b\n", None),
        Captured::NewGeneration(_)
    ));
    assert_eq!(head(&store).stamp, None, "a racy capture keeps no stamp");

    let restored = head(&store);
    assert_ne!(
        restored.stamp,
        Some(settled),
        "the growth check must not report the restore unchanged"
    );
    assert!(matches!(
        capture(&mut store, b"a\n", Some(settled)),
        Captured::NewGeneration(_)
    ));
    assert_eq!(head(&store).generation.0, 3);
    assert_eq!(head(&store).stamp, Some(settled));

    assert!(matches!(
        capture(&mut store, b"a\n", None),
        Captured::Unchanged(_)
    ));
    assert_eq!(
        head(&store).stamp,
        None,
        "an unchanged racy capture clears the stamp too"
    );
}

#[test]
fn a_stamp_is_kept_with_each_capture_and_a_rebuild_forgets_it() {
    let scratch = Scratch::new("stamp");
    let layout = scratch.layout();
    let stamp = |seconds| bilbo_store::entries::Stamp {
        size: 4,
        modified: std::time::UNIX_EPOCH + std::time::Duration::from_nanos(seconds),
    };
    let mut store = open(&layout);
    let label = capture_label("a.md");
    let capture = |store: &mut bilbo_store::segments::Store, bytes: &[u8], at| {
        store
            .capture(&Capture {
                label: &label,
                kind: "markdown",
                id: None,
                id_label: None,
                stamp: Some(stamp(at)),
                bytes,
                ends: &crate::scratch::ends(bytes),
            })
            .expect("capture")
    };
    let head = |store: &bilbo_store::segments::Store| {
        store
            .by_capture(&label)
            .expect("head")
            .and_then(|head| head.stamp)
    };
    capture(&mut store, b"one\n", 1_700_000_000_123_456_789);
    assert_eq!(head(&store), Some(stamp(1_700_000_000_123_456_789)));
    assert!(matches!(
        capture(&mut store, b"one\n", 1_700_000_001_000_000_000),
        Captured::Unchanged(_)
    ));
    assert_eq!(head(&store), Some(stamp(1_700_000_001_000_000_000)));
    capture(&mut store, b"one\ntwo\n", 1_700_000_002_000_000_000);
    assert_eq!(head(&store), Some(stamp(1_700_000_002_000_000_000)));
    store.close().expect("close");

    fs::remove_file(catalog(&layout)).expect("drop the catalog");
    let store = open(&layout);
    assert_eq!(head(&store), None);
}

#[test]
fn changes_from_a_cursor_ahead_of_the_catalog_give_the_catalogs_newest_sequence_number() {
    let scratch = Scratch::new("changes_ahead");
    let layout = scratch.layout();
    let mut store = open(&layout);

    let (none, empty) = store.changes(&Cursor::start()).expect("changes");
    assert!(none.is_empty());
    assert_eq!(empty.seq(), 0);
    let ahead = Cursor::at(empty.catalog(), 100);
    let (none, newest) = store.changes(&ahead).expect("changes");
    assert!(none.is_empty());
    assert_eq!(newest.seq(), 0, "a catalog with no change is at 0");
    assert_eq!(newest.catalog(), empty.catalog());

    keep(&mut store, "a.md", b"one\n");
    keep(&mut store, "b.md", b"two\n");
    let (all, cursor) = store.changes(&Cursor::start()).expect("changes");
    assert!(!all.is_empty());
    let newest_seq = cursor.seq();
    assert!(newest_seq > 0);

    let (none, same) = store.changes(&cursor).expect("changes");
    assert!(none.is_empty());
    assert_eq!(same.seq(), newest_seq, "a current cursor stays");

    let ahead = Cursor::at(cursor.catalog(), newest_seq + 100);
    let (none, real) = store.changes(&ahead).expect("changes");
    assert!(none.is_empty(), "nothing follows a cursor ahead");
    assert_eq!(real.seq(), newest_seq, "the real newest number comes back");
    assert_eq!(real.catalog(), cursor.catalog());
}

#[test]
fn a_capture_label_an_entry_holds_cannot_be_taken_by_another() {
    let scratch = Scratch::new("capture_label_held");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let held = id_of(&keep(&mut store, "a.md", b"one\n"));
    let other = id_of(&keep(&mut store, "b.md", b"two\n"));
    let mut meta = store.meta(other).expect("meta").expect("some");
    meta.labels.capture = Some(capture_label("a.md"));
    match store.set_meta(other, &meta) {
        Err(StoreError::CaptureHeld { label, by }) => {
            assert_eq!(label, capture_label("a.md"));
            assert_eq!(by, held);
        }
        other => panic!("expected CaptureHeld, got {other:?}"),
    }
}
