use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bilbo_store::entries::{Change, Cursor, Entries, Generation, Meta, RecordRange, StoreError};
use bilbo_store::paths::Layout;
use bilbo_store::segments::{Hook, Notice, Options, Step, Store};

use crate::frames::{Summary, frames};
use crate::scratch::{Scratch, catalog, flip, id_of, keep, open, open_seg, open_with, sealed};

pub fn stopping_at(target: Step) -> Hook {
    Box::new(move |step| {
        if step == target {
            Err(io::Error::other("stop"))
        } else {
            Ok(())
        }
    })
}

fn with_hook(layout: &Layout, hook: Hook) -> Result<Store, StoreError> {
    Store::open(
        layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    )
}

pub struct Before {
    pub changes: Vec<Change>,
    pub frames: Vec<Summary>,
    pub reads: Vec<(bilbo_store::id::Id, Vec<u8>)>,
}

/// Three captures and a `set_meta` in one open segment, left unsealed.
pub fn fill(layout: &Layout, hook: Option<Hook>) -> (Store, Before) {
    let mut store = open_with(
        layout,
        Options {
            hook,
            ..Options::default()
        },
    );
    let a = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    keep(&mut store, "e.md", b"");
    keep(&mut store, "a.md", b"one\ntwo\nthree\n");
    let meta = Meta {
        labels: vec!["changed".into()],
        ..store.meta(a).expect("meta").expect("some")
    };
    store.set_meta(a, &meta).expect("set_meta");
    let before = Before {
        changes: store.changes(&Cursor::start()).expect("changes").0,
        frames: frames(&open_seg(layout)),
        reads: vec![(a, b"one\ntwo\nthree\n".to_vec())],
    };
    (store, before)
}

fn all_frames(layout: &Layout) -> Vec<Summary> {
    let mut found = Vec::new();
    for segment in sealed(layout) {
        found.extend(frames(&segment));
    }
    found.extend(frames(&open_seg(layout)));
    found
}

fn assert_whole(layout: &Layout, before: &Before) {
    let store = open(layout);
    let (changes, _) = store.changes(&Cursor::start()).expect("changes");
    assert_eq!(changes, before.changes, "changes after the reopen");
    for (id, bytes) in &before.reads {
        let read = store
            .read(*id, Generation(1), RecordRange { first: 1, count: 3 })
            .expect("read");
        assert_eq!(&read.concat(), bytes);
    }
    drop(store);
    assert_eq!(
        all_frames(layout),
        before.frames,
        "every frame exactly once"
    );

    fs::remove_file(catalog(layout)).expect("delete the catalog");
    let store = open(layout);
    assert_eq!(
        store.changes(&Cursor::start()).expect("changes").0,
        before.changes
    );
    drop(store);

    let read_only = Store::open_read_only(layout, &mut |_| {}).expect("read-only");
    assert_eq!(
        read_only.changes(&Cursor::start()).expect("changes").0,
        before.changes
    );
}

#[test]
fn closing_seals_open_seg_into_one_ulid_segment() {
    let scratch = Scratch::new("closing_seals");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    keep(&mut store, "b.md", b"two\n");
    let bytes = fs::read(open_seg(&layout)).expect("read");
    store.close().expect("close");
    let segments = sealed(&layout);
    assert_eq!(segments.len(), 1);
    let name = segments[0]
        .file_stem()
        .expect("stem")
        .to_string_lossy()
        .to_string();
    assert!(bilbo_store::id::Id::parse(&name).is_some(), "{name}");
    assert_eq!(fs::read(&segments[0]).expect("read"), bytes);
    assert_eq!(fs::metadata(open_seg(&layout)).expect("open.seg").len(), 12);
    assert!(!layout.state.join("sealing").exists());
}

#[test]
fn open_seg_is_sealed_when_it_reaches_the_threshold() {
    let scratch = Scratch::new("threshold");
    let layout = scratch.layout();
    let mut store = open_with(
        &layout,
        Options {
            seal_at: 1,
            ..Options::default()
        },
    );
    for (count, name) in ["a.md", "b.md", "c.md"].iter().enumerate() {
        keep(&mut store, name, b"x\n");
        assert_eq!(sealed(&layout).len(), count + 1);
        assert_eq!(fs::metadata(open_seg(&layout)).expect("open.seg").len(), 12);
    }
}

#[test]
fn segment_names_rise_with_seal_order() {
    let scratch = Scratch::new("names_rise");
    let layout = scratch.layout();
    let mut store = open_with(
        &layout,
        Options {
            seal_at: 1,
            ..Options::default()
        },
    );
    let mut ids = Vec::new();
    for index in 0..20 {
        ids.push(id_of(&keep(&mut store, &format!("{index}.md"), b"x\n")));
    }
    let names: Vec<String> = sealed(&layout)
        .iter()
        .map(|path| {
            path.file_stem()
                .expect("stem")
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(names.len(), 20);
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted, names);
    for (index, path) in sealed(&layout).iter().enumerate() {
        let summaries = frames(path);
        assert_eq!(
            summaries[0].1,
            ids[index].to_bytes(),
            "segment {index} holds capture {index}"
        );
    }
}

#[test]
fn closing_an_empty_open_seg_seals_nothing() {
    let scratch = Scratch::new("close_empty");
    let layout = scratch.layout();
    open(&layout).close().expect("close");
    assert!(sealed(&layout).is_empty());
    assert_eq!(fs::metadata(open_seg(&layout)).expect("open.seg").len(), 12);
}

#[test]
fn a_crash_at_any_seal_step_leaves_every_frame_exactly_once() {
    for target in [
        Step::SealMarked,
        Step::SealCopied,
        Step::SealRenamed,
        Step::SealSynced,
        Step::SealCataloged,
        Step::SealCut,
    ] {
        let scratch = Scratch::new("seal_crash");
        let layout = scratch.layout();
        let (store, before) = fill(&layout, Some(stopping_at(target)));
        let stopped = store.close();
        assert!(
            matches!(&stopped, Err(StoreError::Stopped(step)) if *step == format!("{target:?}")),
            "{target:?}: {stopped:?}"
        );
        assert_whole(&layout, &before);
    }
}

#[test]
fn a_crash_inside_recovery_leaves_every_frame_exactly_once() {
    let scratch = Scratch::new("recovery_crash");
    let layout = scratch.layout();
    let (store, before) = fill(&layout, Some(stopping_at(Step::SealRenamed)));
    assert!(store.close().is_err());
    for target in [Step::RecoverySynced, Step::RecoveryCut] {
        let stopped = with_hook(&layout, stopping_at(target));
        assert!(
            matches!(&stopped, Err(StoreError::Stopped(step)) if *step == format!("{target:?}")),
            "{target:?}"
        );
    }
    assert_whole(&layout, &before);
}

#[test]
fn recovery_syncs_the_root_before_it_cuts_open_seg() {
    let scratch = Scratch::new("recovery_order");
    let layout = scratch.layout();
    let (store, before) = fill(&layout, Some(stopping_at(Step::SealRenamed)));
    assert!(store.close().is_err());
    let full = fs::metadata(open_seg(&layout)).expect("open.seg").len();

    let seen: Arc<Mutex<Vec<(Step, u64)>>> = Arc::default();
    let log = Arc::clone(&seen);
    let path: PathBuf = open_seg(&layout);
    let hook: Hook = Box::new(move |step| {
        let length = fs::metadata(&path).map_or(0, |meta| meta.len());
        log.lock().expect("log").push((step, length));
        Ok(())
    });
    let store = with_hook(&layout, hook).expect("open");
    drop(store);
    let seen = seen.lock().expect("log").clone();
    let recovery: Vec<_> = seen
        .iter()
        .filter(|(step, _)| matches!(step, Step::RecoverySynced | Step::RecoveryCut))
        .copied()
        .collect();
    assert_eq!(
        recovery.iter().map(|(step, _)| *step).collect::<Vec<_>>(),
        vec![Step::RecoverySynced, Step::RecoveryCut]
    );
    assert_eq!(
        recovery[0].1, full,
        "open.seg is whole when the root is synced"
    );
    assert_eq!(recovery[1].1, 12, "open.seg is cut by the next step");
    assert_whole(&layout, &before);
}

#[test]
fn a_sealed_segment_that_does_not_match_open_seg_stops_recovery_and_keeps_both() {
    for target in [Step::SealRenamed, Step::SealCut] {
        let scratch = Scratch::new("mismatch");
        let layout = scratch.layout();
        let (store, _) = fill(&layout, Some(stopping_at(target)));
        assert!(store.close().is_err());
        let segment = sealed(&layout).pop().expect("sealed");
        let last = fs::metadata(&segment).expect("segment").len() as usize - 1;
        flip(&segment, last);
        let marker = layout.state.join("sealing");
        let held = (
            fs::read(&marker).expect("marker"),
            fs::read(open_seg(&layout)).expect("open.seg"),
            fs::read(&segment).expect("segment"),
        );
        match Store::open(&layout, Options::default(), &mut |_: Notice| {}) {
            Err(StoreError::Damaged { .. }) => {}
            Ok(_) => panic!("{target:?}: the open must fail"),
            Err(other) => panic!("{target:?}: expected Damaged, got {other}"),
        }
        assert_eq!(
            held,
            (
                fs::read(&marker).expect("marker"),
                fs::read(open_seg(&layout)).expect("open.seg"),
                fs::read(&segment).expect("segment"),
            ),
            "{target:?}"
        );
    }
}

#[test]
fn a_read_only_open_during_an_interrupted_seal_sees_every_frame_once() {
    for target in [
        Step::SealMarked,
        Step::SealCopied,
        Step::SealRenamed,
        Step::SealSynced,
        Step::SealCataloged,
        Step::SealCut,
    ] {
        let scratch = Scratch::new("read_only_mid_seal");
        let layout = scratch.layout();
        let (store, before) = fill(&layout, Some(stopping_at(target)));
        assert!(store.close().is_err());
        let held = crate::scratch::snapshot_of(&layout);

        let reader = Store::open_read_only(&layout, &mut |_| {}).expect("read-only open");
        assert_eq!(
            reader.changes(&Cursor::start()).expect("changes").0,
            before.changes,
            "{target:?}"
        );
        for (id, bytes) in &before.reads {
            let read = reader
                .read(*id, Generation(1), RecordRange { first: 1, count: 3 })
                .expect("read");
            assert_eq!(&read.concat(), bytes, "{target:?}");
        }
        drop(reader);
        assert_eq!(crate::scratch::snapshot_of(&layout), held, "{target:?}");
    }
}

#[test]
fn a_read_only_open_refuses_a_sealed_segment_that_differs_from_open_seg() {
    let scratch = Scratch::new("read_only_mismatch");
    let layout = scratch.layout();
    let (store, _) = fill(&layout, Some(stopping_at(Step::SealRenamed)));
    assert!(store.close().is_err());
    let segment = sealed(&layout).pop().expect("sealed");
    let last = fs::metadata(&segment).expect("segment").len() as usize - 1;
    flip(&segment, last);
    let held = crate::scratch::snapshot_of(&layout);
    match Store::open_read_only(&layout, &mut |_| {}) {
        Err(StoreError::Damaged { path, .. }) => assert_eq!(path, segment),
        Ok(_) => panic!("the open must fail"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }
    assert_eq!(crate::scratch::snapshot_of(&layout), held);
}
