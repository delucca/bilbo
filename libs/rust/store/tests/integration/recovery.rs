use std::fs;
use std::io::Write;

use bilbo_store::entries::{Captured, Entries, Generation, RecordRange, StoreError};
use bilbo_store::limits::TearBudget;
use bilbo_store::segments::{Notice, Options, Rebuild, Store};

use crate::frames::{frames, spans};
use crate::scratch::{
    Scratch, catalog, flip, id_of, keep, open, open_seg, origin, read_records, sealed, sealing,
};

fn reopen(layout: &bilbo_store::layout::Layout) -> Result<(Store, Vec<Notice>), StoreError> {
    let mut notices = Vec::new();
    Store::open(layout, sealing(), &mut |notice| {
        notices.push(notice);
    })
    .map(|store| (store, notices))
}

fn corrupt_length(path: &std::path::Path, at: usize) {
    let mut bytes = fs::read(path).expect("read");
    bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(path, bytes).expect("write");
}

#[test]
fn a_torn_tail_of_open_seg_is_cut_when_the_store_opens() {
    let scratch = Scratch::new("torn_tail");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    drop(store);
    let whole = fs::read(open_seg(&layout)).expect("read");
    let first = &whole[12..];
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(open_seg(&layout))
        .expect("open");
    file.write_all(&first[..40]).expect("append");
    drop(file);

    let (store, notices) = reopen(&layout).expect("open");
    assert_eq!(
        notices,
        vec![Notice::CutTornTail {
            path: open_seg(&layout),
            offset: whole.len() as u64,
            bytes: 40
        }]
    );
    assert_eq!(fs::read(open_seg(&layout)).expect("read"), whole);
    assert_eq!(read_records(&store, id, 1, 2), b"one\ntwo\n");
}

#[test]
fn a_corrupt_last_frame_of_open_seg_is_cut() {
    let scratch = Scratch::new("corrupt_last");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    keep(&mut store, "b.md", b"two\n");
    drop(store);
    let last = spans(&open_seg(&layout)).pop().expect("a frame");
    flip(&open_seg(&layout), last.offset + last.total - 1);

    let (store, notices) = reopen(&layout).expect("open");
    assert_eq!(
        notices.first(),
        Some(&Notice::CutTornTail {
            path: open_seg(&layout),
            offset: last.offset as u64,
            bytes: last.total as u64
        })
    );
    assert_eq!(
        fs::metadata(open_seg(&layout)).expect("open.seg").len(),
        last.offset as u64
    );
    assert_eq!(read_records(&store, a, 1, 1), b"one\n");
}

fn three_captures(name: &str) -> (Scratch, bilbo_store::layout::Layout) {
    let scratch = Scratch::new(name);
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    keep(&mut store, "b.md", b"two\n");
    keep(&mut store, "c.md", b"three\n");
    drop(store);
    (scratch, layout)
}

fn assert_refused_and_untouched(layout: &bilbo_store::layout::Layout, offset: usize) {
    let segment = fs::read(open_seg(layout)).expect("read");
    let database = fs::read(catalog(layout)).expect("read");
    match reopen(layout) {
        Err(StoreError::Damaged {
            path, offset: at, ..
        }) => {
            assert_eq!(path, open_seg(layout));
            assert_eq!(at, offset as u64);
        }
        Ok(_) => panic!("the open must fail"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }
    assert_eq!(fs::read(open_seg(layout)).expect("read"), segment);
    assert_eq!(fs::read(catalog(layout)).expect("read"), database);
}

#[test]
fn a_damaged_middle_frame_of_open_seg_refuses_the_open_and_keeps_every_byte() {
    let (_scratch, layout) = three_captures("damaged_middle");
    let second_body = &spans(&open_seg(&layout))[3];
    flip(
        &open_seg(&layout),
        second_body.offset + second_body.total - 1,
    );
    assert_refused_and_untouched(&layout, second_body.offset);
}

#[test]
fn a_corrupted_length_in_a_middle_frame_refuses_the_open_and_keeps_every_byte() {
    let (_scratch, layout) = three_captures("corrupt_middle_length");
    let second_body = &spans(&open_seg(&layout))[3];
    corrupt_length(&open_seg(&layout), second_body.offset);
    assert_refused_and_untouched(&layout, second_body.offset);
}

#[test]
fn a_crash_between_the_fsync_and_the_catalog_commit_is_caught_up() {
    let scratch = Scratch::new("crash_before_commit");
    let layout = scratch.layout();
    let hook: bilbo_store::segments::Hook = Box::new(|step| {
        if step == bilbo_store::segments::Step::Appended {
            Err(std::io::Error::other("stop"))
        } else {
            Ok(())
        }
    });
    let mut store = open_with_hook(&layout, hook);
    let stopped = crate::scratch::try_keep(&mut store, "a.md", b"one\ntwo\n", None);
    assert!(matches!(
        stopped,
        Err(bilbo_store::entries::CaptureError::Store(StoreError::Stopped(ref step)))
            if step == "Appended"
    ));
    drop(store);

    let (mut store, _) = reopen(&layout).expect("open");
    let head = store
        .by_origin(&origin("a.md"))
        .expect("head")
        .expect("caught up");
    assert_eq!(head.records, 2);
    assert_eq!(
        keep(&mut store, "a.md", b"one\ntwo\n"),
        Captured::Unchanged(head.id)
    );
    assert_eq!(read_records(&store, head.id, 1, 2), b"one\ntwo\n");
}

fn open_with_hook(
    layout: &bilbo_store::layout::Layout,
    hook: bilbo_store::segments::Hook,
) -> Store {
    crate::scratch::open_with(
        layout,
        Options {
            hook: Some(hook),
            ..sealing()
        },
    )
}

#[test]
fn a_deleted_catalog_is_rebuilt_from_the_segments_and_open_seg() {
    let scratch = Scratch::new("rebuild");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    store.close().expect("close");
    let mut store = open(&layout);
    let b = id_of(&keep(&mut store, "b.md", b"three\n"));
    keep(&mut store, "a.md", b"one\ntwo\nmore\n");
    let heads = (
        store.by_origin(&origin("a.md")).expect("head"),
        store.by_origin(&origin("b.md")).expect("head"),
    );
    drop(store);
    fs::remove_file(catalog(&layout)).expect("delete");

    let (store, notices) = reopen(&layout).expect("open");
    assert_eq!(
        notices,
        vec![Notice::RebuiltCatalog {
            entries: 2,
            segments: 1
        }]
    );
    assert_eq!(
        (
            store.by_origin(&origin("a.md")).expect("head"),
            store.by_origin(&origin("b.md")).expect("head")
        ),
        heads
    );
    assert_eq!(read_records(&store, a, 1, 3), b"one\ntwo\nmore\n");
    assert_eq!(read_records(&store, b, 1, 1), b"three\n");
}

#[test]
fn a_catalog_that_is_not_a_database_is_rebuilt() {
    let scratch = Scratch::new("garbage_catalog");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    drop(store);
    fs::write(
        catalog(&layout),
        b"this is not a database at all, not even close",
    )
    .expect("write");

    let (store, notices) = reopen(&layout).expect("open");
    assert_eq!(
        notices,
        vec![Notice::RebuiltCatalog {
            entries: 1,
            segments: 0
        }]
    );
    assert_eq!(read_records(&store, a, 1, 1), b"one\n");
}

#[test]
fn a_damaged_sealed_segment_fails_a_read_and_a_rebuild_and_is_left_as_it_is() {
    let scratch = Scratch::new("damaged_sealed");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    store.close().expect("close");
    let segment = sealed(&layout).pop().expect("sealed");
    let last = fs::metadata(&segment).expect("segment").len() as usize - 1;
    flip(&segment, last);
    let damaged = fs::read(&segment).expect("read");

    let (store, _) = reopen(&layout).expect("the open does not read sealed segments");
    let read = store.read(a, Generation(1), RecordRange { first: 1, count: 2 });
    assert!(matches!(read, Err(StoreError::Damaged { .. })), "{read:?}");
    drop(store);
    assert_eq!(fs::read(&segment).expect("read"), damaged);

    fs::remove_file(catalog(&layout)).expect("delete");
    match reopen(&layout) {
        Err(StoreError::Damaged { path, .. }) => assert_eq!(path, segment),
        Ok(_) => panic!("the rebuild must fail"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }
    assert_eq!(fs::read(&segment).expect("read"), damaged);
}

#[test]
fn a_segment_of_a_newer_format_is_refused() {
    let scratch = Scratch::new("newer_format");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    let segment = sealed(&layout).pop().expect("sealed");
    let mut bytes = fs::read(&segment).expect("read");
    bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
    fs::write(&segment, &bytes).expect("write");
    fs::remove_file(catalog(&layout)).expect("delete");
    match reopen(&layout) {
        Err(StoreError::Newer { path, version }) => {
            assert_eq!(path, segment);
            assert_eq!(version, 2);
        }
        Ok(_) => panic!("must be refused"),
        Err(other) => panic!("expected Newer, got {other}"),
    }
    assert_eq!(fs::read(&segment).expect("read"), bytes);
}

#[test]
fn an_entry_whose_body_frame_was_torn_away_is_new_on_the_next_capture() {
    let scratch = Scratch::new("torn_body");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    drop(store);
    let body = spans(&open_seg(&layout)).pop().expect("body frame");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(open_seg(&layout))
        .expect("open");
    file.set_len((body.offset + 30) as u64).expect("cut");
    drop(file);

    let (mut store, notices) = reopen(&layout).expect("open");
    assert!(matches!(notices.first(), Some(Notice::CutTornTail { .. })));
    let head = store
        .by_origin(&origin("a.md"))
        .expect("head")
        .expect("meta survives");
    assert_eq!(head.id, id);
    assert_eq!(head.generation, Generation(0));
    let Captured::New(kept) = keep(&mut store, "a.md", b"one\ntwo\n") else {
        panic!("expected New");
    };
    assert_eq!(kept.id, id);
    assert_eq!(kept.generation, Generation(1));
    assert_eq!(read_records(&store, id, 1, 2), b"one\ntwo\n");
    let kinds: Vec<u8> = frames(&open_seg(&layout)).iter().map(|f| f.0).collect();
    assert_eq!(kinds, vec![1, 2]);
}

fn stopping_at(target: bilbo_store::segments::Step) -> bilbo_store::segments::Hook {
    Box::new(move |step| {
        if step == target {
            Err(std::io::Error::other("stop"))
        } else {
            Ok(())
        }
    })
}

#[test]
fn a_double_fault_around_a_good_frame_refuses_the_open_and_keeps_every_byte() {
    let scratch = Scratch::new("double_fault");
    let layout = scratch.layout();
    let mut store = open(&layout);
    for name in ["a.md", "b.md", "c.md", "d.md"] {
        keep(&mut store, name, b"one\n");
    }
    drop(store);
    let all = spans(&open_seg(&layout));
    let damaged = &all[3];
    flip(&open_seg(&layout), damaged.offset + damaged.total - 1);
    let last = all.last().expect("a frame");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(open_seg(&layout))
        .expect("open");
    file.set_len((last.offset + 40) as u64).expect("tear");
    drop(file);
    assert_refused_and_untouched(&layout, damaged.offset);
}

#[test]
fn a_new_capture_stopped_between_its_two_syncs_is_completed_by_the_next_one() {
    use bilbo_store::segments::Step;
    let scratch = Scratch::new("meta_synced");
    let layout = scratch.layout();
    let mut store = open_with_hook(&layout, stopping_at(Step::MetaSynced));
    let stopped = crate::scratch::try_keep(&mut store, "a.md", b"one\ntwo\n", None);
    assert!(matches!(
        stopped,
        Err(bilbo_store::entries::CaptureError::Store(StoreError::Stopped(ref step)))
            if step == "MetaSynced"
    ));
    drop(store);
    let kinds: Vec<u8> = frames(&open_seg(&layout)).iter().map(|f| f.0).collect();
    assert_eq!(kinds, vec![1]);

    let (mut store, notices) = reopen(&layout).expect("open");
    assert!(notices.is_empty());
    let head = store
        .by_origin(&origin("a.md"))
        .expect("head")
        .expect("meta caught up");
    assert_eq!(head.generation, Generation(0));
    let Captured::New(kept) = keep(&mut store, "a.md", b"one\ntwo\n") else {
        panic!("expected New");
    };
    assert_eq!(kept.id, head.id);
    assert_eq!(read_records(&store, head.id, 1, 2), b"one\ntwo\n");
    let kinds: Vec<u8> = frames(&open_seg(&layout)).iter().map(|f| f.0).collect();
    assert_eq!(kinds, vec![1, 2]);
}

#[test]
fn a_stopped_store_refuses_more_writes_and_seals_nothing() {
    use bilbo_store::segments::Step;
    for target in [Step::MetaSynced, Step::Appended] {
        let scratch = Scratch::new("poisoned");
        let layout = scratch.layout();
        let mut store = open_with_hook(&layout, stopping_at(target));
        assert!(crate::scratch::try_keep(&mut store, "a.md", b"one\n", None).is_err());
        let again = crate::scratch::try_keep(&mut store, "b.md", b"two\n", None);
        assert!(
            matches!(
                again,
                Err(bilbo_store::entries::CaptureError::Store(
                    StoreError::Poisoned
                ))
            ),
            "{target:?}: {again:?}"
        );
        let meta = bilbo_store::entries::Meta {
            id: bilbo_store::id::Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id"),
            kind: "k".into(),
            origin: None,
            captured: std::time::UNIX_EPOCH,
            parent: None,
        };
        assert!(matches!(
            store.set_meta(meta.id, &meta),
            Err(StoreError::Poisoned)
        ));
        let length = fs::metadata(open_seg(&layout)).expect("open.seg").len();
        store.close().expect("a poisoned close seals nothing");
        assert!(sealed(&layout).is_empty(), "{target:?}");
        assert_eq!(
            fs::metadata(open_seg(&layout)).expect("open.seg").len(),
            length
        );
    }
}

#[test]
fn a_body_payload_that_is_not_what_its_frame_says_fails_the_rebuild_and_changes_nothing() {
    let valid_plain = |extra: &[u8]| {
        let mut plain = vec![4u8, 4];
        plain.extend_from_slice(b"one\ntwo\n");
        plain.extend_from_slice(extra);
        crate::frames::compress(&plain)
    };
    let mut table_too_long = vec![5u8];
    table_too_long.push(b'a');
    let mut one_length_for_two = vec![4u8];
    one_length_for_two.extend_from_slice(b"one\n");
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "a length past the payload",
            crate::frames::compress(&table_too_long),
        ),
        (
            "fewer lengths than records",
            crate::frames::compress(&one_length_for_two),
        ),
        ("bytes beyond the records", valid_plain(b"z")),
        ("not a zstd frame", b"not zstd at all".to_vec()),
    ];
    for (name, payload) in cases {
        let scratch = Scratch::new("bad_body_sealed");
        let layout = scratch.layout();
        let mut store = open(&layout);
        keep(&mut store, "a.md", b"one\ntwo\n");
        store.close().expect("close");
        let segment = sealed(&layout).pop().expect("sealed");
        let body = spans(&segment).pop().expect("body frame");
        let raw = fs::read(&segment).expect("read");
        let forged = crate::frames::forge(&raw[body.offset..body.offset + body.total], &payload);
        crate::frames::replace(&segment, &body, &forged);
        let held = fs::read(&segment).expect("read");
        fs::remove_file(catalog(&layout)).expect("delete");
        match reopen(&layout) {
            Err(StoreError::Damaged { path, offset, .. }) => {
                assert_eq!(path, segment, "{name}");
                assert_eq!(offset, body.offset as u64, "{name}");
            }
            Ok(_) => panic!("{name}: the rebuild must fail"),
            Err(other) => panic!("{name}: expected Damaged, got {other}"),
        }
        assert_eq!(fs::read(&segment).expect("read"), held, "{name}");
        assert_eq!(sealed(&layout), vec![segment], "{name}");
    }
}

#[test]
fn a_count_0_body_with_a_payload_fails_the_rebuild() {
    let scratch = Scratch::new("bad_empty_body");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "e.md", b"");
    store.close().expect("close");
    let segment = sealed(&layout).pop().expect("sealed");
    let body = spans(&segment).pop().expect("body frame");
    let raw = fs::read(&segment).expect("read");
    let forged = crate::frames::forge(
        &raw[body.offset..body.offset + body.total],
        &crate::frames::compress(b"x"),
    );
    crate::frames::replace(&segment, &body, &forged);
    fs::remove_file(catalog(&layout)).expect("delete");
    assert!(matches!(reopen(&layout), Err(StoreError::Damaged { .. })));
}

#[test]
fn a_bad_body_payload_in_open_seg_fails_the_catch_up_and_the_rebuild_behind_it() {
    let scratch = Scratch::new("bad_body_open");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    drop(store);
    let behind = fs::read(catalog(&layout)).expect("read");
    let mut store = open(&layout);
    keep(&mut store, "b.md", b"two\nthree\n");
    drop(store);
    fs::write(catalog(&layout), behind).expect("restore the older catalog");
    let body = spans(&open_seg(&layout)).pop().expect("body frame");
    let raw = fs::read(open_seg(&layout)).expect("read");
    let forged = crate::frames::forge(
        &raw[body.offset..body.offset + body.total],
        &crate::frames::compress(b"\x09zz"),
    );
    crate::frames::replace(&open_seg(&layout), &body, &forged);
    let held = fs::read(open_seg(&layout)).expect("read");
    match reopen(&layout) {
        Err(StoreError::Damaged { offset, .. }) => assert_eq!(offset, body.offset as u64),
        Ok(_) => panic!("the catch-up must fail"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }
    assert_eq!(fs::read(open_seg(&layout)).expect("read"), held);
}

#[test]
fn a_read_checks_the_frame_against_the_catalog_row_that_named_it() {
    let scratch = Scratch::new("swapped_frames");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"aa\n"));
    keep(&mut store, "b.md", b"bb\n");
    drop(store);
    let all = spans(&open_seg(&layout));
    let (first, second) = (&all[1], &all[3]);
    assert_eq!(first.total, second.total, "the swap needs equal frames");
    let raw = fs::read(open_seg(&layout)).expect("read");
    let mut swapped = raw.clone();
    swapped[first.offset..first.offset + first.total]
        .copy_from_slice(&raw[second.offset..second.offset + second.total]);
    swapped[second.offset..second.offset + second.total]
        .copy_from_slice(&raw[first.offset..first.offset + first.total]);
    fs::write(open_seg(&layout), swapped).expect("write");

    let (store, _) = reopen(&layout).expect("open");
    let read = store.read(a, Generation(1), RecordRange { first: 1, count: 1 });
    assert!(matches!(read, Err(StoreError::Damaged { .. })), "{read:?}");
}

#[test]
fn a_torn_marker_is_removed_when_the_store_opens() {
    let scratch = Scratch::new("torn_marker");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    drop(store);
    let marker = layout.state.join("sealing");
    fs::write(&marker, b"01JC5Q3V7X").expect("write");
    let (store, _) = reopen(&layout).expect("open");
    assert!(!marker.exists());
    assert_eq!(read_records(&store, id, 1, 1), b"one\n");
}

#[test]
fn a_catalog_offset_that_is_not_a_frame_boundary_falls_back_to_a_rebuild() {
    let behind = Scratch::new("offset_behind");
    let behind_layout = behind.layout();
    let mut store = open(&behind_layout);
    keep(&mut store, "a.md", b"x\n");
    drop(store);
    let stale = fs::read(catalog(&behind_layout)).expect("read");
    let applied = fs::metadata(open_seg(&behind_layout))
        .expect("open.seg")
        .len() as usize;

    let scratch = Scratch::new("offset_mid_frame");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(
        &mut store,
        "b.md",
        b"a longer note than the other one\n",
    ));
    drop(store);
    assert!(
        spans(&open_seg(&layout))
            .iter()
            .all(|s| s.offset != applied),
        "the stale offset must fall inside a frame"
    );
    fs::write(catalog(&layout), stale).expect("write");

    let (store, notices) = reopen(&layout).expect("open");
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, Notice::RebuiltCatalog { .. })),
        "{notices:?}"
    );
    assert_eq!(
        read_records(&store, id, 1, 1),
        b"a longer note than the other one\n"
    );
    assert!(store.by_origin(&origin("a.md")).expect("head").is_none());
}

#[test]
fn a_torn_header_is_rewritten_and_reported() {
    let scratch = Scratch::new("torn_header");
    let layout = scratch.layout();
    drop(open(&layout));
    fs::write(open_seg(&layout), &b"BILBO"[..]).expect("write");
    let (_store, notices) = reopen(&layout).expect("open");
    assert_eq!(
        notices,
        vec![Notice::CutTornTail {
            path: open_seg(&layout),
            offset: 0,
            bytes: 5
        }]
    );
    assert_eq!(fs::metadata(open_seg(&layout)).expect("open.seg").len(), 12);
}

/// A segment header, then zeros, with eight frame headers that pass their sum and each claim a
/// frame to the end of the 8 MiB file: hashing them all costs just under eight times the file.
fn crafted_tear() -> Vec<u8> {
    use xxhash_rust::xxh3::Xxh3;

    let len = 8 * 1024 * 1024;
    let mut bytes = vec![0u8; len];
    bytes[..8].copy_from_slice(b"BILBOSEG");
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    for slot in 0..8 {
        let at = 100 + slot * 77;
        let mut header = [0u8; 77];
        header[..4].copy_from_slice(&u32::try_from(len - at - 4).expect("length").to_le_bytes());
        let mut hasher = Xxh3::new();
        hasher.update(&header[..4]);
        hasher.update(&header[16..]);
        header[4..8].copy_from_slice(&(hasher.digest() as u32).to_le_bytes());
        bytes[at..at + 77].copy_from_slice(&header);
    }
    bytes
}

fn budget(times: usize) -> Options {
    Options {
        tear_budget: TearBudget::try_from(times).expect("in range"),
        ..sealing()
    }
}

#[test]
fn a_reader_applies_the_configured_tear_budget() {
    let scratch = Scratch::new("budget_reader");
    let layout = scratch.layout();
    drop(open(&layout));
    let crafted = crafted_tear();
    fs::write(open_seg(&layout), &crafted).expect("write");

    let enough = Store::open_read_only(&layout, budget(8), &mut |_| {});
    assert!(enough.is_ok(), "{:?}", enough.err());
    drop(enough);
    let too_small = Store::open_read_only(&layout, budget(7), &mut |_| {});
    assert!(
        matches!(too_small, Err(StoreError::Damaged { offset: 12, .. })),
        "{:?}",
        too_small.err()
    );
    assert_eq!(fs::read(open_seg(&layout)).expect("read"), crafted);
}

#[test]
fn a_writer_applies_the_configured_tear_budget() {
    let scratch = Scratch::new("budget_writer");
    let layout = scratch.layout();
    drop(open(&layout));
    let crafted = crafted_tear();
    fs::write(open_seg(&layout), &crafted).expect("write");

    let too_small = Store::open(&layout, budget(7), &mut |_| {});
    assert!(
        matches!(too_small, Err(StoreError::Damaged { offset: 12, .. })),
        "{:?}",
        too_small.err()
    );
    assert_eq!(fs::read(open_seg(&layout)).expect("read"), crafted);

    let mut notices = Vec::new();
    let enough = Store::open(&layout, budget(8), &mut |notice| notices.push(notice));
    assert!(enough.is_ok(), "{:?}", enough.err());
    assert_eq!(
        notices,
        vec![Notice::CutTornTail {
            path: open_seg(&layout),
            offset: 12,
            bytes: crafted.len() as u64 - 12
        }]
    );
    assert_eq!(fs::metadata(open_seg(&layout)).expect("open.seg").len(), 12);
}

#[test]
fn a_store_that_may_not_rebuild_refuses_a_missing_or_stale_catalog_and_keeps_nothing() {
    let scratch = Scratch::new("unrebuilt");
    let layout = scratch.layout();
    let no_rebuild = || Options {
        rebuild: Rebuild::CatchUpOnly,
        ..sealing()
    };
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    let old = fs::read(catalog(&layout)).expect("the catalog of one segment");

    fs::remove_file(catalog(&layout)).expect("remove the catalog");
    assert!(matches!(
        Store::open(&layout, no_rebuild(), &mut |_| {}),
        Err(StoreError::Unrebuilt)
    ));
    assert!(!catalog(&layout).exists(), "nothing rebuilt");

    let mut store = open(&layout);
    keep(&mut store, "b.md", b"two\n");
    store.close().expect("close");
    assert_eq!(sealed(&layout).len(), 2);
    let open_seg_before = fs::read(open_seg(&layout)).expect("open.seg");
    fs::write(catalog(&layout), &old).expect("a catalog that lists one segment of two");
    assert!(matches!(
        Store::open(&layout, no_rebuild(), &mut |_| {}),
        Err(StoreError::Unrebuilt)
    ));
    assert_eq!(fs::read(catalog(&layout)).expect("catalog"), old);
    assert_eq!(
        fs::read(open_seg(&layout)).expect("open.seg"),
        open_seg_before
    );

    let store = open(&layout);
    assert!(
        store.by_origin(&origin("b.md")).expect("head").is_some(),
        "a run that may rebuilds"
    );
    store.close().expect("close");
}

#[test]
fn a_catch_up_after_an_interrupted_capture_drops_the_stale_stamp() {
    use bilbo_store::entries::{Capture, Stamp};
    let scratch = Scratch::new("catch_up_stamp");
    let layout = scratch.layout();
    let settled = Stamp {
        size: 2,
        modified: std::time::UNIX_EPOCH
            + std::time::Duration::from_nanos(1_700_000_000_000_000_000),
    };
    let origin = origin("a.md");
    let capture = |store: &mut Store, bytes: &[u8], stamp: Option<Stamp>| {
        store.capture(&Capture {
            origin: &origin,
            kind: "markdown",
            id: None,
            parent: None,
            stamp,
            bytes,
            ends: &crate::scratch::ends(bytes),
        })
    };

    let mut store = open(&layout);
    capture(&mut store, b"a\n", Some(settled)).expect("capture");
    assert_eq!(
        store
            .by_origin(&origin)
            .expect("head")
            .expect("entry")
            .stamp,
        Some(settled)
    );
    drop(store);

    let mut store = open_with_hook(&layout, stopping_at(bilbo_store::segments::Step::Appended));
    assert!(capture(&mut store, b"b\n", None).is_err());
    drop(store);

    let (mut store, _) = reopen(&layout).expect("open");
    let head = store.by_origin(&origin).expect("head").expect("entry");
    assert_eq!(head.generation.0, 2, "the catch-up recovered the capture");
    assert_eq!(head.stamp, None, "the stale stamp must not survive");

    let restored = capture(&mut store, b"a\n", Some(settled)).expect("capture");
    assert!(
        matches!(restored, Captured::NewGeneration(_)),
        "{restored:?}"
    );
    let head = store.by_origin(&origin).expect("head").expect("entry");
    assert_eq!(head.generation.0, 3);
    assert_eq!(head.stamp, Some(settled));
}
