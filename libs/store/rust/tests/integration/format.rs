use std::fs;
use std::path::Path;

use bilbo_store::entries::{Captured, Entries, Generation, RecordRange, StoreError};
use bilbo_store::labels::{By, Labels, Standing, Tag, Tags};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Notice, Options, Step, Store};

use crate::frames::{frames, spans};
use crate::recovery::reopen;
use crate::scratch::{
    Scratch, capture_label, catalog, id_of, keep, keep_tagged, open, open_seg, read_records,
    sealed, sealing,
};
use crate::seal::{all_frames, stopping_at};

const HEADER_1: [u8; 12] = *b"BILBOSEG\x01\0\0\0";
const HEADER_2: [u8; 12] = *b"BILBOSEG\x02\0\0\0";

fn set_format(path: &Path, format: u32) {
    let mut bytes = fs::read(path).expect("read");
    bytes[8..12].copy_from_slice(&format.to_le_bytes());
    fs::write(path, bytes).expect("write");
}

fn three(layout: &Layout) -> Vec<bilbo_store::id::Id> {
    let mut store = open(layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    let b = id_of(&keep(&mut store, "b.md", b"three\n"));
    drop(store);
    vec![a, b]
}

fn format_1_with_frames(name: &str) -> (Scratch, Layout, Vec<bilbo_store::id::Id>, Vec<u8>) {
    let scratch = Scratch::new(name);
    let layout = scratch.layout();
    let ids = three(&layout);
    set_format(&open_seg(&layout), 1);
    let bytes = fs::read(open_seg(&layout)).expect("read");
    (scratch, layout, ids, bytes)
}

fn assert_reads(store: &Store, ids: &[bilbo_store::id::Id]) {
    assert_eq!(read_records(store, ids[0], 1, 2), b"one\ntwo\n");
    assert_eq!(read_records(store, ids[1], 1, 1), b"three\n");
}

#[test]
fn a_format_1_open_segment_with_frames_is_sealed_once_at_the_first_writable_open() {
    let (_scratch, layout, ids, before) = format_1_with_frames("format_1_frames");
    let store = open(&layout);
    let segments = sealed(&layout);
    assert_eq!(segments.len(), 1);
    assert_eq!(fs::read(&segments[0]).expect("read"), before);
    assert_eq!(&before[..12], &HEADER_1);
    assert_eq!(fs::read(open_seg(&layout)).expect("open.seg"), HEADER_2);
    assert_reads(&store, &ids);
    drop(store);
    drop(open(&layout));
    assert_eq!(sealed(&layout).len(), 1);
}

#[test]
fn a_format_1_open_segment_with_no_frame_gets_the_new_header_and_no_seal() {
    let scratch = Scratch::new("format_1_empty");
    let layout = scratch.layout();
    drop(open(&layout));
    set_format(&open_seg(&layout), 1);
    drop(open(&layout));
    assert!(sealed(&layout).is_empty());
    assert_eq!(fs::read(open_seg(&layout)).expect("open.seg"), HEADER_2);
}

#[test]
fn a_format_2_open_segment_with_frames_is_not_sealed_by_an_open() {
    let scratch = Scratch::new("format_2_frames");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    drop(store);
    let before = fs::read(open_seg(&layout)).expect("read");
    let store = Store::open(&layout, Options::default(), &mut |_| {}).expect("open");
    drop(store);
    assert!(sealed(&layout).is_empty());
    assert_eq!(fs::read(open_seg(&layout)).expect("read"), before);
}

#[test]
fn a_stop_at_each_step_of_the_first_open_is_repaired_by_the_next() {
    for target in [
        Step::SealMarked,
        Step::SealCopied,
        Step::SealRenamed,
        Step::SealSynced,
        Step::SealCataloged,
        Step::SealCut,
    ] {
        let (_scratch, layout, ids, _) = format_1_with_frames("format_1_stop");
        let expected = frames(&open_seg(&layout));
        let stopped = Store::open(
            &layout,
            Options {
                hook: Some(stopping_at(target)),
                ..sealing()
            },
            &mut |_| {},
        );
        assert!(
            matches!(&stopped, Err(StoreError::Stopped(step)) if *step == format!("{target:?}")),
            "{target:?}"
        );
        drop(stopped);
        let store = open(&layout);
        assert_reads(&store, &ids);
        drop(store);
        assert_eq!(all_frames(&layout), expected, "{target:?}");
        assert!(
            fs::read(open_seg(&layout))
                .expect("read")
                .starts_with(&HEADER_2)
        );
        assert!(!layout.state.join("sealing").exists(), "{target:?}");
    }

    for left in [&HEADER_1[..], &HEADER_2[..5]] {
        let (_scratch, layout, ids, _) = format_1_with_frames("format_1_after_seal");
        let expected = frames(&open_seg(&layout));
        drop(open(&layout));
        fs::write(open_seg(&layout), left).expect("write");
        let store = open(&layout);
        assert_reads(&store, &ids);
        drop(store);
        assert_eq!(sealed(&layout).len(), 1, "{}", left.len());
        assert_eq!(fs::read(open_seg(&layout)).expect("read"), HEADER_2);
        assert_eq!(all_frames(&layout), expected, "{}", left.len());
    }
}

#[test]
fn a_torn_header_of_either_format_is_written_again_by_a_writable_open() {
    for header in [HEADER_1, HEADER_2] {
        let scratch = Scratch::new("torn_header");
        let layout = scratch.layout();
        drop(open(&layout));
        fs::write(open_seg(&layout), &header[..9]).expect("write");
        let (store, notices) = reopen(&layout).expect("open");
        drop(store);
        assert!(
            notices
                .iter()
                .any(|notice| matches!(notice, Notice::CutTornTail { offset: 0, .. })),
            "{notices:?}"
        );
        assert_eq!(fs::read(open_seg(&layout)).expect("read"), HEADER_2);
    }
}

#[test]
fn a_sealed_segment_with_a_torn_header_is_damage_and_a_rebuild_files_a_torn_open_segment_as_empty()
{
    let scratch = Scratch::new("torn_sealed");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    let segment = sealed(&layout).pop().expect("sealed");
    fs::write(&segment, &HEADER_1[..9]).expect("write");
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    match reopen(&layout) {
        Err(StoreError::Damaged { what, .. }) => assert_eq!(what, "the header is cut short"),
        Ok(_) => panic!("must be refused"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }

    let scratch = Scratch::new("torn_open");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\n"));
    drop(store);
    fs::write(open_seg(&layout), &HEADER_1[..9]).expect("write");
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only");
    assert_eq!(store.meta(id).expect("meta"), None);
    assert_eq!(
        store.by_capture(&capture_label("a.md")).expect("head"),
        None
    );
}

fn tagged_entry(layout: &Layout, close: bool) -> bilbo_store::id::Id {
    let mut tags = Tags::default();
    tags.set(
        Tag::new("a").expect("tag"),
        Some(Standing::Carried(By::Person)),
    );
    let mut store = open(layout);
    let id = id_of(&keep_tagged(
        &mut store, "a.md", "markdown", b"one\n", &tags,
    ));
    if close {
        store.close().expect("close");
    }
    id
}

fn assert_tag_damage(layout: &Layout, path: &Path) {
    let offset = spans(path)[0].offset;
    match reopen(layout) {
        Err(StoreError::Damaged {
            path: at,
            offset: found,
            what,
        }) => {
            assert_eq!(at, path);
            assert_eq!(found, offset as u64);
            assert_eq!(what, "a tag stands in a format 1 segment");
        }
        Ok(_) => panic!("must be refused"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }
}

#[test]
fn a_tag_under_a_format_1_header_is_damage_in_each_walk() {
    let scratch = Scratch::new("tag_format_1_sealed");
    let layout = scratch.layout();
    tagged_entry(&layout, true);
    let segment = sealed(&layout).pop().expect("sealed");
    set_format(&segment, 1);
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    assert_tag_damage(&layout, &segment);

    let scratch = Scratch::new("tag_format_1_open");
    let layout = scratch.layout();
    tagged_entry(&layout, false);
    set_format(&open_seg(&layout), 1);
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    assert_tag_damage(&layout, &open_seg(&layout));

    let scratch = Scratch::new("tag_format_1_catch_up");
    let layout = scratch.layout();
    drop(open(&layout));
    let saved = layout.cache.with_file_name("saved-catalog");
    fs::copy(catalog(&layout), &saved).expect("copy the catalog");
    tagged_entry(&layout, false);
    set_format(&open_seg(&layout), 1);
    fs::copy(&saved, catalog(&layout)).expect("put the catalog back");
    assert_tag_damage(&layout, &open_seg(&layout));
}

fn assert_untouched_refusal(layout: &Layout) {
    let path = open_seg(layout);
    let before = fs::read(&path).expect("read");
    assert_tag_damage(layout, &path);
    assert!(sealed(layout).is_empty(), "nothing is sealed");
    assert_eq!(
        fs::read(&path).expect("read"),
        before,
        "open.seg is as it was"
    );
}

#[test]
fn a_tag_under_a_format_1_header_is_damage_though_the_catalog_holds_its_frame() {
    let scratch = Scratch::new("tag_format_1_kept_catalog");
    let layout = scratch.layout();
    tagged_entry(&layout, false);
    set_format(&open_seg(&layout), 1);
    assert_untouched_refusal(&layout);

    let scratch = Scratch::new("tag_format_1_behind_catalog");
    let layout = scratch.layout();
    tagged_entry(&layout, false);
    let saved = layout.cache.with_file_name("saved-catalog");
    fs::copy(catalog(&layout), &saved).expect("copy the catalog");
    let mut store = open(&layout);
    keep(&mut store, "b.md", b"two\n");
    drop(store);
    fs::copy(&saved, catalog(&layout)).expect("put the catalog back");
    set_format(&open_seg(&layout), 1);
    assert_untouched_refusal(&layout);
}

#[test]
fn a_reader_takes_no_kept_catalog_over_an_older_open_segment_that_holds_frames() {
    let scratch = Scratch::new("tag_format_1_reader");
    let layout = scratch.layout();
    let id = tagged_entry(&layout, false);
    set_format(&open_seg(&layout), 1);
    let path = open_seg(&layout);
    let offset = spans(&path)[0].offset;
    match Store::open_read_only(&layout, sealing(), &mut |_| {}) {
        Err(StoreError::Damaged {
            path: at,
            offset: found,
            what,
        }) => {
            assert_eq!(at, path);
            assert_eq!(found, offset as u64);
            assert_eq!(what, "a tag stands in a format 1 segment");
        }
        Ok(_) => panic!("must be refused"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }
    set_format(&path, 2);
    let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only");
    let meta = store.meta(id).expect("meta").expect("a meta");
    assert_eq!(
        meta.labels.tags.standing(&Tag::new("a").expect("tag")),
        Some(Standing::Carried(By::Person))
    );
}

#[test]
fn a_format_3_segment_is_newer_and_the_message_names_both_formats() {
    let scratch = Scratch::new("format_3");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    let segment = sealed(&layout).pop().expect("sealed");
    set_format(&segment, 3);
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    match reopen(&layout) {
        Err(error @ StoreError::Newer { .. }) => assert_eq!(
            error.to_string(),
            format!(
                "{} is segment format 3; this bilbo reads formats 1 and 2",
                segment.display()
            )
        ),
        Ok(_) => panic!("must be refused"),
        Err(other) => panic!("expected Newer, got {other}"),
    }
}

#[test]
fn a_capture_after_the_header_is_written_again_lands_after_it_and_reads_back() {
    let scratch = Scratch::new("header_again");
    let layout = scratch.layout();
    drop(open(&layout));
    fs::write(open_seg(&layout), HEADER_1).expect("write");
    let mut store = open(&layout);
    let captured = keep(&mut store, "a.md", b"one\ntwo\nthree");
    let Captured::New(new) = captured else {
        panic!("expected New, got {captured:?}");
    };
    drop(store);
    let whole = fs::read(open_seg(&layout)).expect("open.seg");
    assert_eq!(whole[..12], HEADER_2);
    assert_eq!(whole.len() as u64, 12 + new.stored_bytes);
    assert_eq!(frames(&open_seg(&layout)).len(), 2);
    let store = open(&layout);
    assert_eq!(read_records(&store, new.id, 1, 3), b"one\ntwo\nthree");
}

#[test]
fn a_format_1_store_reads_as_it_did() {
    let scratch = Scratch::new("format_1_reads");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let above = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    let label = capture_label("b.md");
    let below = id_of(
        &store
            .capture(&bilbo_store::entries::Capture {
                label: &label,
                kind: "markdown",
                id: None,
                id_label: Some(above),
                tags: &Tags::default(),
                stamp: None,
                bytes: b"three\n",
                ends: &[6],
            })
            .expect("capture"),
    );
    let metas = [
        store.meta(above).expect("meta"),
        store.meta(below).expect("meta"),
    ];
    let head = store.by_capture(&capture_label("a.md")).expect("head");
    store.close().expect("close");
    let segment = sealed(&layout).pop().expect("sealed");
    set_format(&segment, 1);
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let store = open(&layout);
    assert_eq!(
        [
            store.meta(above).expect("meta"),
            store.meta(below).expect("meta")
        ],
        metas
    );
    for meta in &metas {
        assert!(meta.as_ref().expect("some").labels.tags.is_empty());
    }
    assert_eq!(
        store.by_capture(&capture_label("a.md")).expect("head"),
        head
    );
    assert_eq!(store.belonging_to(&[above]).expect("found"), vec![below]);
    assert_eq!(
        store
            .read(above, Generation(1), RecordRange { first: 1, count: 2 })
            .expect("read")
            .concat(),
        b"one\ntwo\n"
    );
}
