use std::fs;

use bilbo_store::entries::{CaptureError, Entries, Generation, Meta, RecordRange, StoreError};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Cache, Options, Step, Store};

use crate::frames::spans;
use crate::scratch::{
    Scratch, catalog, chmod, id_of, keep, open, open_seg, open_with, origin, read_records, sealing,
    snapshot, snapshot_of, sql, try_keep,
};
use crate::seal::stopping_at;

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

fn reader(layout: &Layout) -> Store {
    Store::open_read_only(layout, sealing(), &mut |_| {}).expect("read-only open")
}

fn linked(target: &std::path::Path, name: &str) -> std::path::PathBuf {
    let link = target.with_file_name(name);
    std::os::unix::fs::symlink(target, &link).expect("link");
    link
}

#[test]
fn a_read_only_open_gives_its_own_cache_canonical() {
    let scratch = Scratch::new("reader_cache_own");
    let layout = scratch.layout();
    two_places_cataloged(&layout);
    let canonical = fs::canonicalize(&layout.cache).expect("canonical");

    assert_eq!(
        reader(&layout).reader_cache(),
        &Cache::Own(canonical.clone())
    );
    let through_link = Layout {
        cache: linked(&layout.cache, "cache-link"),
        ..layout.clone()
    };
    assert_eq!(reader(&through_link).reader_cache(), &Cache::Own(canonical));
}

#[test]
fn a_store_with_no_lock_file_offers_its_reader_no_cache() {
    let scratch = Scratch::new("reader_cache_unlocked");
    let layout = scratch.layout();
    let (a, _) = two_places_cataloged(&layout);
    let lock = layout.state.join("store.lock");
    fs::remove_file(&lock).expect("delete the lock file");
    let before = snapshot_of(&layout);

    let store = reader(&layout);
    assert_eq!(store.reader_cache(), &Cache::Unlocked);
    assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n");
    drop(store);
    assert!(!lock.exists());
    assert_eq!(snapshot_of(&layout), before);
}

#[test]
fn a_cache_bound_to_another_store_is_not_this_readers() {
    let scratch = Scratch::new("reader_cache_other");
    let ours = scratch.layout();
    two_places_cataloged(&ours);
    let theirs = Layout {
        root: ours.root.with_file_name("root-b"),
        state: ours.state.with_file_name("state-b"),
        cache: ours.cache.with_file_name("cache-b"),
    };
    let mut store = open(&theirs);
    keep(&mut store, "z.md", b"other\n");
    store.close().expect("close");
    let held = snapshot(&theirs.cache);
    let listing = fs::read_dir(&theirs.cache).expect("list").count();
    let canonical = fs::canonicalize(&theirs.cache).expect("canonical");

    let direct = Layout {
        cache: theirs.cache.clone(),
        ..ours.clone()
    };
    let through_link = Layout {
        cache: linked(&theirs.cache, "cache-b-link"),
        ..ours.clone()
    };
    for layout in [direct, through_link] {
        let store = reader(&layout);
        assert_eq!(store.reader_cache(), &Cache::OtherStore(canonical.clone()));
        assert!(store.by_origin(&origin("a.md")).expect("head").is_some());
        drop(store);
        assert_eq!(snapshot(&theirs.cache), held);
        assert_eq!(fs::read_dir(&theirs.cache).expect("list").count(), listing);
    }
}

#[test]
fn a_cache_with_no_catalog_or_one_of_another_version_has_no_catalog() {
    type Spoil = fn(&Layout);
    let cases: [(&str, Spoil); 6] = [
        ("no catalog", |layout| {
            fs::remove_file(catalog(layout)).expect("delete");
        }),
        ("another version", |layout| {
            sql(layout, "PRAGMA user_version = 3");
        }),
        ("not a database", |layout| {
            fs::write(catalog(layout), b"not sqlite").expect("write");
        }),
        ("a file of 50 bytes", |layout| {
            let bytes = fs::read(catalog(layout)).expect("read");
            fs::write(catalog(layout), &bytes[..50]).expect("write");
        }),
        ("the magic and another user_version", |layout| {
            let mut bytes = fs::read(catalog(layout)).expect("read");
            bytes[60..64].copy_from_slice(&3u32.to_be_bytes());
            fs::write(catalog(layout), bytes).expect("write");
        }),
        ("no binding rows", |layout| {
            sql(layout, "DELETE FROM catalog WHERE key IN ('root', 'state')");
        }),
    ];
    for (case, spoil) in cases {
        let scratch = Scratch::new("reader_cache_no_catalog");
        let layout = scratch.layout();
        let (a, _) = two_places_cataloged(&layout);
        spoil(&layout);
        let canonical = fs::canonicalize(&layout.cache).expect("canonical");
        let store = reader(&layout);
        assert_eq!(store.reader_cache(), &Cache::NoCatalog(canonical), "{case}");
        assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n", "{case}");
    }
}

#[test]
fn a_stale_catalog_of_this_store_is_still_this_stores() {
    let scratch = Scratch::new("reader_cache_stale");
    let layout = scratch.layout();
    let (a, _) = two_places_cataloged(&layout);
    let mut store = open_with(
        &layout,
        Options {
            hook: Some(stopping_at(Step::Appended)),
            ..sealing()
        },
    );
    let cut = try_keep(&mut store, "c.md", b"late\n", None);
    assert!(cut.is_err(), "the writer stopped before its catalog commit");
    drop(store);

    let store = reader(&layout);
    assert_eq!(
        store.reader_cache(),
        &Cache::Own(fs::canonicalize(&layout.cache).expect("canonical"))
    );
    let late = store
        .by_origin(&origin("c.md"))
        .expect("head")
        .expect("the segments hold it");
    assert_eq!(read_records(&store, late.id, 1, 1), b"late\n");
    assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n");
}

#[test]
fn a_read_only_open_refuses_a_cache_others_can_write() {
    for mode in [0o777, 0o707] {
        let scratch = Scratch::new("reader_cache_unsafe");
        let layout = scratch.layout();
        let (a, _) = two_places_cataloged(&layout);
        chmod(&layout.cache, mode);
        let canonical = fs::canonicalize(&layout.cache).expect("canonical");

        let store = reader(&layout);
        assert_eq!(store.reader_cache(), &Cache::Unsafe(canonical), "{mode:o}");
        assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n", "{mode:o}");
        drop(store);
        chmod(&layout.cache, 0o700);
    }
}

#[test]
fn a_read_only_open_with_no_cache_folder_says_absent_and_creates_none() {
    let scratch = Scratch::new("reader_cache_absent");
    let layout = scratch.layout();
    two_places_cataloged(&layout);
    fs::remove_dir_all(&layout.cache).expect("remove the cache");
    let before = snapshot_of(&layout);

    let store = reader(&layout);
    assert_eq!(store.reader_cache(), &Cache::Absent);
    drop(store);
    assert!(!layout.cache.exists());
    assert_eq!(snapshot_of(&layout), before);
}

#[test]
fn a_writers_store_always_owns_its_cache() {
    let scratch = Scratch::new("reader_cache_writer");
    let layout = scratch.layout();
    let store = open(&layout);
    assert_eq!(
        store.reader_cache(),
        &Cache::Own(store.cache().to_path_buf())
    );
    assert_eq!(
        store.cache(),
        fs::canonicalize(&layout.cache).expect("canonical")
    );
}

#[test]
fn the_kept_catalog_is_still_used_when_the_cache_is_judged_once() {
    let scratch = Scratch::new("reader_cache_kept");
    let layout = scratch.layout();
    let (a, b) = two_places_cataloged(&layout);
    let segment = crate::scratch::sealed(&layout).pop().expect("a segment");
    let frame = spans(&segment).pop().expect("a frame");
    crate::scratch::flip(&segment, frame.offset + frame.total - 1);

    let store = reader(&layout);
    assert!(matches!(store.reader_cache(), Cache::Own(_)));
    assert_eq!(read_records(&store, b, 1, 1), b"three\n");
    assert!(
        matches!(
            store.read(a, Generation(1), RecordRange { first: 1, count: 2 }),
            Err(StoreError::Damaged { .. })
        ),
        "a walk of the segments would have refused the open"
    );
}

fn same_folder(held: &fs::File, path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let held = held.metadata().expect("meta");
    let found = fs::metadata(path).expect("meta");
    (held.dev(), held.ino()) == (found.dev(), found.ino())
}

#[test]
fn a_reader_of_a_store_with_no_state_folder_is_given_no_state() {
    let scratch = Scratch::new("reader_state");
    let layout = scratch.layout();
    two_places_cataloged(&layout);
    let canonical = fs::canonicalize(&layout.state).expect("canonical");
    let store = reader(&layout);
    let state = store.state().expect("a state");
    assert_eq!(state.path(), canonical);
    assert!(same_folder(state.folder(), &canonical));
    drop(store);
    let through_link = Layout {
        state: linked(&layout.state, "state-link"),
        ..layout.clone()
    };
    let store = reader(&through_link);
    let state = store.state().expect("a state");
    assert_eq!(state.path(), canonical);
    assert!(same_folder(state.folder(), &canonical));
    drop(store);

    assert!(open(&layout).state().is_none(), "a writer's store has none");
    fs::remove_dir_all(&layout.state).expect("remove the state folder");
    assert!(reader(&layout).state().is_none());
    assert!(!layout.state.exists());
}

#[test]
fn a_readers_state_is_the_folder_it_checked_not_the_path() {
    let scratch = Scratch::new("reader_state_handle");
    let layout = scratch.layout();
    two_places_cataloged(&layout);
    let canonical = fs::canonicalize(&layout.state).expect("canonical");
    let store = reader(&layout);
    let aside = canonical.with_file_name("state-aside");
    fs::rename(&canonical, &aside).expect("rename the folder aside");
    fs::create_dir(&canonical).expect("another folder at the path");
    chmod(&canonical, 0o700);

    let state = store.state().expect("a state");
    assert_eq!(state.path(), canonical);
    assert!(same_folder(state.folder(), &aside));
    assert!(!same_folder(state.folder(), &canonical));
}

fn make_wal(layout: &Layout) {
    let mut bytes = fs::read(catalog(layout)).expect("read");
    bytes[18] = 2;
    bytes[19] = 2;
    fs::write(catalog(layout), bytes).expect("write");
}

#[test]
fn a_wal_mode_catalog_is_not_opened_and_gets_no_sidecar() {
    let scratch = Scratch::new("reader_cache_wal");
    let ours = scratch.layout();
    let (a, b) = two_places_cataloged(&ours);
    make_wal(&ours);
    let canonical = fs::canonicalize(&ours.cache).expect("canonical");
    let held = snapshot(&ours.cache);

    let store = reader(&ours);
    assert_eq!(store.reader_cache(), &Cache::NoCatalog(canonical.clone()));
    assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n");
    assert_eq!(read_records(&store, b, 1, 1), b"three\n");
    drop(store);
    assert_eq!(snapshot(&ours.cache), held);

    let theirs = Layout {
        root: ours.root.with_file_name("root-b"),
        state: ours.state.with_file_name("state-b"),
        cache: ours.cache.with_file_name("cache-b"),
    };
    let mut store = open(&theirs);
    keep(&mut store, "z.md", b"other\n");
    store.close().expect("close");
    make_wal(&theirs);
    let held = snapshot(&theirs.cache);
    let pointed = Layout {
        cache: theirs.cache.clone(),
        ..ours.clone()
    };
    let store = reader(&pointed);
    assert_eq!(
        store.reader_cache(),
        &Cache::NoCatalog(fs::canonicalize(&theirs.cache).expect("canonical"))
    );
    drop(store);
    assert_eq!(snapshot(&theirs.cache), held);
}

#[test]
fn an_ancestor_of_the_cache_alone_open_to_others_makes_the_cache_unsafe() {
    let scratch = Scratch::new("reader_cache_ancestor");
    let base = scratch.layout();
    let loose = base.root.with_file_name("loose");
    fs::create_dir(&loose).expect("loose folder");
    chmod(&loose, 0o700);
    let layout = Layout {
        cache: loose.join("cache"),
        ..base
    };
    two_places_cataloged(&layout);
    chmod(&loose, 0o777);

    let store = reader(&layout);
    assert_eq!(store.reader_cache(), &Cache::Unsafe(loose.clone()));
    drop(store);
    chmod(&loose, 0o700);
}

#[test]
fn a_wal_file_beside_a_rollback_catalog_is_not_opened_and_gets_no_shm() {
    let scratch = Scratch::new("reader_cache_wal_file");
    let layout = scratch.layout();
    let (a, b) = two_places_cataloged(&layout);
    let wal = layout.cache.join("catalog.sqlite-wal");
    let id_before = catalog_id(&layout);
    fs::write(&wal, [0u8; 32]).expect("a wal file");
    let held = snapshot(&layout.cache);

    let store = reader(&layout);
    assert_eq!(
        store.reader_cache(),
        &Cache::NoCatalog(fs::canonicalize(&layout.cache).expect("canonical"))
    );
    assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n");
    assert_eq!(read_records(&store, b, 1, 1), b"three\n");
    drop(store);
    assert_eq!(snapshot(&layout.cache), held);

    open(&layout).close().expect("close");
    let names: Vec<_> = fs::read_dir(&layout.cache)
        .expect("list")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert!(
        names
            .iter()
            .all(|name| name != "catalog.sqlite-wal" && name != "catalog.sqlite-shm"),
        "{names:?}"
    );
    assert_ne!(catalog_id(&layout), id_before, "the catalog was made again");
}

fn catalog_id(layout: &Layout) -> Vec<u8> {
    rusqlite::Connection::open(catalog(layout))
        .expect("open")
        .query_row("SELECT value FROM catalog WHERE key = 'id'", [], |row| {
            row.get(0)
        })
        .expect("id")
}

fn link_catalog(layout: &Layout, with_wal: bool) {
    let real = layout.cache.join("real.sqlite");
    fs::rename(catalog(layout), &real).expect("rename");
    std::os::unix::fs::symlink("real.sqlite", catalog(layout)).expect("link");
    if with_wal {
        fs::write(layout.cache.join("real.sqlite-wal"), [0u8; 32]).expect("a wal file");
    }
}

#[test]
fn a_catalog_that_is_a_link_is_no_catalog_and_its_target_gets_no_shm() {
    for with_wal in [true, false] {
        let scratch = Scratch::new("reader_cache_linked_catalog");
        let layout = scratch.layout();
        let (a, b) = two_places_cataloged(&layout);
        link_catalog(&layout, with_wal);
        let held = snapshot(&layout.cache);

        let store = reader(&layout);
        assert_eq!(
            store.reader_cache(),
            &Cache::NoCatalog(fs::canonicalize(&layout.cache).expect("canonical")),
            "wal: {with_wal}"
        );
        assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n");
        assert_eq!(read_records(&store, b, 1, 1), b"three\n");
        drop(store);
        assert_eq!(snapshot(&layout.cache), held, "wal: {with_wal}");

        if with_wal {
            open(&layout).close().expect("close");
            assert!(
                fs::symlink_metadata(catalog(&layout))
                    .expect("meta")
                    .is_file(),
                "the link was replaced by a file"
            );
            assert!(matches!(reader(&layout).reader_cache(), Cache::Own(_)));
            let after = snapshot(&layout.cache);
            for name in ["real.sqlite", "real.sqlite-wal"] {
                let path = layout.cache.join(name);
                assert_eq!(after.get(&path), held.get(&path), "{name}");
            }
            assert!(!layout.cache.join("real.sqlite-shm").exists());
        }
    }
}

#[test]
fn a_catalog_whose_hot_journal_restores_another_version_is_rebuilt() {
    let scratch = Scratch::new("reader_cache_hot_journal");
    let layout = scratch.layout();
    let (a, b) = two_places_cataloged(&layout);
    let journal = layout.cache.join("catalog.sqlite-journal");
    let saved = layout.cache.with_file_name("saved-journal");
    {
        let conn = rusqlite::Connection::open(catalog(&layout)).expect("open");
        conn.execute_batch("PRAGMA synchronous = OFF; PRAGMA user_version = 3;")
            .expect("version 3");
        conn.execute_batch("BEGIN; PRAGMA user_version = 2;")
            .expect("begin");
        fs::copy(&journal, &saved).expect("the journal of the open transaction");
        conn.execute_batch("COMMIT;").expect("commit");
    }
    fs::copy(&saved, &journal).expect("put the journal back");
    let held = snapshot(&layout.cache);

    let store = reader(&layout);
    assert_eq!(
        store.reader_cache(),
        &Cache::NoCatalog(fs::canonicalize(&layout.cache).expect("canonical"))
    );
    assert_eq!(read_records(&store, a, 1, 2), b"one\ntwo\n");
    drop(store);
    assert_eq!(snapshot(&layout.cache), held);
    assert!(journal.exists());

    let mut store = open(&layout);
    assert_eq!(read_records(&store, b, 1, 1), b"three\n");
    keep(&mut store, "c.md", b"x\n");
    store.close().expect("close");
    let version: i64 = rusqlite::Connection::open(catalog(&layout))
        .expect("open")
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2, "the catalog was made again");
}
