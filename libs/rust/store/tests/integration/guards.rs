use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use bilbo_store::entries::{CaptureError, Captured, Entries, Generation, RecordRange, StoreError};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Hook, Notice, Options, Step, Store};

use crate::frames::{frames, spans};
use crate::scratch::{
    Scratch, catalog, chmod, id_of, keep, open, open_seg, open_with, origin, read_records, sealed,
    snapshot, snapshot_of, sql, try_keep,
};
use crate::seal::{fill, stopping_at};

const WAIT: Duration = Duration::from_secs(10);

fn second_root(first: &Layout) -> Layout {
    Layout {
        root: first.root.with_file_name("root2"),
        state: first.state.clone(),
        cache: first.cache.with_file_name("cache2"),
    }
}

#[test]
fn a_catalog_is_never_trusted_by_another_store_that_shares_its_cache_folder() {
    let scratch = Scratch::new("shared_cache");
    let one = scratch.layout();
    let two = Layout {
        root: one.root.with_file_name("root2"),
        state: one.state.with_file_name("state2"),
        cache: one.cache.clone(),
    };
    let mut store = open(&one);
    let first = id_of(&keep(&mut store, "a.md", b"x\n"));
    drop(store);
    let mut store = open(&two);
    let second = id_of(&keep(&mut store, "a.md", b"x\n"));
    drop(store);
    assert_ne!(first, second);

    for (layout, id) in [(&one, first), (&two, second)] {
        let store = open(layout);
        let head = store
            .by_origin(&origin("a.md"))
            .expect("head")
            .expect("entry");
        assert_eq!(head.id, id);
        assert_eq!(read_records(&store, id, 1, 1), b"x\n");
    }
}

#[test]
fn a_waiting_reader_checks_the_root_binding_again_under_its_lock() {
    let scratch = Scratch::new("reader_rebinds");
    let first = scratch.layout();
    let mut store = open(&first);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    let record = first.state.join("root");
    let bound = fs::read(&record).expect("the root record");
    fs::remove_file(&record).expect("remove");
    let second = second_root(&first);
    fs::create_dir_all(&second.root).expect("root");

    let holder = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(first.state.join("store.lock"))
        .expect("lock file");
    holder.lock().expect("hold the lock");
    let (waiting_tx, waiting_rx) = channel::<Notice>();
    let (done_tx, done_rx) = channel::<Option<String>>();
    let layout = second.clone();
    let reader = thread::spawn(move || {
        let result = Store::open_read_only(&layout, Options::default(), &mut |notice| {
            waiting_tx.send(notice).ok();
        });
        done_tx
            .send(result.err().map(|error| error.to_string()))
            .ok();
    });
    assert_eq!(waiting_rx.recv_timeout(WAIT), Ok(Notice::Waiting));
    fs::write(&record, bound).expect("a writer binds the state folder meanwhile");
    drop(holder);
    let error = done_rx
        .recv_timeout(WAIT)
        .expect("the reader finishes")
        .expect("the reader must fail");
    assert!(error.contains("belongs to the store at"), "{error}");
    reader.join().expect("reader thread");
}

#[test]
fn a_torn_root_record_counts_as_absent_and_a_writer_rewrites_it() {
    for (name, torn) in [
        ("empty", &b""[..]),
        ("cut mid-line", &b"/some/ro"[..]),
        ("a bare newline", &b"\n"[..]),
    ] {
        let scratch = Scratch::new("torn_root");
        let layout = scratch.layout();
        let mut store = open(&layout);
        keep(&mut store, "a.md", b"one\n");
        store.close().expect("close");
        let record = layout.state.join("root");
        let good = fs::read(&record).expect("record");
        fs::write(&record, torn).expect("tear it");

        let reader = Store::open_read_only(&layout, Options::default(), &mut |_| {})
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        drop(reader);
        assert_eq!(
            fs::read(&record).expect("record"),
            torn,
            "{name}: a reader writes nothing"
        );

        let store = Store::open(&layout, Options::default(), &mut |_| {})
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        drop(store);
        assert_eq!(fs::read(&record).expect("record"), good, "{name}");
        assert!(!layout.state.join("root.tmp").exists(), "{name}");
    }
}

#[test]
fn a_folder_that_others_can_write_is_refused_and_left_untouched() {
    for which in ["root", "state", "cache"] {
        for writer in [true, false] {
            if !writer && which == "cache" {
                continue;
            }
            let scratch = Scratch::new("unsafe_folder");
            let layout = scratch.layout();
            let mut store = open(&layout);
            keep(&mut store, "a.md", b"one\n");
            store.close().expect("close");
            let folder = match which {
                "root" => &layout.root,
                "state" => &layout.state,
                _ => &layout.cache,
            };
            chmod(folder, 0o777);
            let held = snapshot_of(&layout);
            let result = if writer {
                Store::open(&layout, Options::default(), &mut |_| {}).err()
            } else {
                Store::open_read_only(&layout, Options::default(), &mut |_| {}).err()
            };
            match result {
                Some(StoreError::Unsafe(path)) => assert_eq!(&path, folder),
                other => panic!("{which} writer={writer}: expected Unsafe, got {other:?}"),
            }
            assert_eq!(snapshot_of(&layout), held, "{which} writer={writer}");
        }
    }
}

#[test]
fn a_writer_refused_for_another_root_creates_no_cache_folder() {
    let scratch = Scratch::new("refused_creates_nothing");
    let first = scratch.layout();
    open(&first).close().expect("close");
    let second = second_root(&first);
    let refused = Store::open(&second, Options::default(), &mut |_| {});
    assert!(matches!(refused, Err(StoreError::OtherRoot { .. })));
    assert!(!second.cache.exists());
}

#[test]
fn a_forged_catalog_row_cannot_steer_a_read() {
    let scratch = Scratch::new("forged_rows");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"x\ny\n"));
    drop(store);
    let range = RecordRange { first: 1, count: 2 };

    sql(&layout, "UPDATE frames SET segment = '../x'");
    let store = open(&layout);
    let read = store.read(id, Generation(1), range);
    assert!(matches!(read, Err(StoreError::Catalog { .. })), "{read:?}");
    drop(store);
    sql(&layout, "UPDATE frames SET segment = 'open'");

    sql(&layout, "UPDATE frames SET length = 1152921504606846976");
    let store = open(&layout);
    let read = store.read(id, Generation(1), range);
    assert!(matches!(read, Err(StoreError::Damaged { .. })), "{read:?}");
}

#[test]
fn a_read_checks_every_field_of_the_frame_against_its_row() {
    let cases = [
        ("count", "UPDATE frames SET count = 3"),
        ("first", "UPDATE frames SET first = 0"),
        (
            "generation",
            "INSERT INTO generations SELECT entry, 2, records, origin_end, origin_hash FROM generations;
             UPDATE frames SET generation = 2",
        ),
    ];
    for (name, statement) in cases {
        let scratch = Scratch::new("row_fields");
        let layout = scratch.layout();
        let mut store = open(&layout);
        let id = id_of(&keep(&mut store, "a.md", b"x\ny\n"));
        drop(store);
        sql(&layout, statement);
        let store = open(&layout);
        let generation = if name == "generation" { 2 } else { 1 };
        let read = store.read(
            id,
            Generation(generation),
            RecordRange { first: 1, count: 2 },
        );
        assert!(
            matches!(read, Err(StoreError::Damaged { .. })),
            "{name}: {read:?}"
        );
    }
}

#[test]
fn frames_that_do_not_cover_the_records_are_a_catalog_error_naming_the_catalog() {
    let scratch = Scratch::new("uncovered");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"x\n"));
    keep(&mut store, "a.md", b"x\ny\n");
    drop(store);
    sql(&layout, "DELETE FROM frames WHERE first = 2");
    let store = open(&layout);
    let read = store.read(id, Generation(1), RecordRange { first: 1, count: 2 });
    match read {
        Err(StoreError::Catalog { path, error }) => {
            assert_eq!(path, catalog(&layout));
            assert_eq!(error, "its frames do not cover the records");
        }
        other => panic!("expected Catalog, got {other:?}"),
    }
}

#[test]
fn a_sealed_copy_that_is_a_valid_shorter_prefix_stops_recovery_and_keeps_everything() {
    let scratch = Scratch::new("shorter_prefix");
    let layout = scratch.layout();
    let (store, _) = fill(&layout, Some(stopping_at(Step::SealRenamed)));
    assert!(store.close().is_err());
    let segment = sealed(&layout).pop().expect("sealed");
    let first = spans(&segment).remove(0);
    let keep_bytes = first.offset + first.total;
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&segment)
        .expect("open");
    file.set_len(keep_bytes as u64).expect("shorten");
    drop(file);
    let held = snapshot_of(&layout);
    match Store::open(&layout, Options::default(), &mut |_| {}) {
        Err(StoreError::Damaged { path, offset, .. }) => {
            assert_eq!(path, segment);
            assert_eq!(offset, keep_bytes as u64);
        }
        Ok(_) => panic!("the open must fail"),
        Err(other) => panic!("expected Damaged, got {other}"),
    }
    assert_eq!(snapshot_of(&layout), held);
}

#[test]
fn files_that_exist_but_cannot_be_opened_are_errors_not_absences() {
    let scratch = Scratch::new("unreadable");
    let layout = scratch.layout();
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");

    let lock = layout.state.join("store.lock");
    chmod(&lock, 0o000);
    let reader = Store::open_read_only(&layout, Options::default(), &mut |_| {});
    chmod(&lock, 0o600);
    assert!(
        matches!(reader, Err(StoreError::Io { action: "lock", .. })),
        "{:?}",
        reader.err()
    );

    let segment = open_seg(&layout);
    let held = fs::read(&segment).expect("read");
    chmod(&segment, 0o000);
    let writer = Store::open(&layout, Options::default(), &mut |_| {});
    chmod(&segment, 0o600);
    assert!(
        matches!(writer, Err(StoreError::Io { action: "read", .. })),
        "{:?}",
        writer.err()
    );
    assert_eq!(fs::read(&segment).expect("read"), held);
}

fn failing_the_write_number(failing: usize, calls: Arc<AtomicUsize>) -> Hook {
    Box::new(move |step| {
        if step == Step::FrameWritten && calls.fetch_add(1, Ordering::SeqCst) + 1 == failing {
            Err(std::io::Error::other("disk full"))
        } else {
            Ok(())
        }
    })
}

#[test]
fn a_failed_first_frame_write_is_rolled_back_and_the_store_stays_usable() {
    let scratch = Scratch::new("rollback_first");
    let layout = scratch.layout();
    let hook = failing_the_write_number(1, Arc::default());
    let mut store = open_with(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
    );
    let failed = try_keep(&mut store, "a.md", b"one\ntwo\n", None);
    assert!(
        matches!(
            failed,
            Err(CaptureError::Store(StoreError::Io {
                action: "write",
                ..
            }))
        ),
        "{failed:?}"
    );
    assert_eq!(fs::metadata(open_seg(&layout)).expect("open.seg").len(), 12);
    let Captured::New(kept) = keep(&mut store, "a.md", b"one\ntwo\n") else {
        panic!("expected New");
    };
    assert_eq!(read_records(&store, kept.id, 1, 2), b"one\ntwo\n");
    let kinds: Vec<u8> = frames(&open_seg(&layout)).iter().map(|f| f.0).collect();
    assert_eq!(kinds, vec![1, 2]);
}

#[test]
fn a_failed_second_frame_write_keeps_the_store_poisoned_and_the_meta_is_completed_later() {
    let scratch = Scratch::new("rollback_second");
    let layout = scratch.layout();
    let hook = failing_the_write_number(2, Arc::default());
    let mut store = open_with(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
    );
    let failed = try_keep(&mut store, "a.md", b"one\n", None);
    assert!(matches!(
        failed,
        Err(CaptureError::Store(StoreError::Io {
            action: "write",
            ..
        }))
    ));
    let kinds: Vec<u8> = frames(&open_seg(&layout)).iter().map(|f| f.0).collect();
    assert_eq!(kinds, vec![1], "only the synced meta frame is left");
    let again = try_keep(&mut store, "b.md", b"two\n", None);
    assert!(matches!(
        again,
        Err(CaptureError::Store(StoreError::Poisoned))
    ));
    drop(store);

    let mut store = open(&layout);
    let Captured::New(kept) = keep(&mut store, "a.md", b"one\n") else {
        panic!("expected New");
    };
    assert_eq!(read_records(&store, kept.id, 1, 1), b"one\n");
}

#[test]
fn a_read_only_open_of_a_torn_open_seg_header_reads_the_sealed_segments() {
    let scratch = Scratch::new("read_only_torn_header");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    store.close().expect("close");
    fs::write(open_seg(&layout), b"BILBO").expect("tear the header");
    let held = snapshot_of(&layout);
    let reader =
        Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("read-only open");
    assert_eq!(read_records(&reader, a, 1, 1), b"one\n");
    drop(reader);
    assert_eq!(snapshot_of(&layout), held);
}

#[test]
fn a_folder_another_user_owns_is_refused_before_anything_is_written() {
    let scratch = Scratch::new("foreign_owner");
    let mut layout = scratch.layout();
    layout.state = std::path::PathBuf::from("/usr");
    let writer = Store::open(&layout, Options::default(), &mut |_| {});
    assert!(
        matches!(&writer, Err(StoreError::Unsafe(path)) if path == &layout.state),
        "{:?}",
        writer.err()
    );
    fs::create_dir_all(&layout.root).expect("root");
    let reader = Store::open_read_only(&layout, Options::default(), &mut |_| {});
    assert!(
        matches!(reader, Err(StoreError::Unsafe(_))),
        "{:?}",
        reader.err()
    );
    assert!(!layout.root.join("store.lock").exists());
}

#[test]
fn a_folder_that_is_open_to_others_when_it_is_first_used_is_refused_and_stays_empty() {
    for which in ["root", "cache"] {
        let scratch = Scratch::new("unsafe_first_use");
        let layout = scratch.layout();
        let folder = if which == "root" {
            &layout.root
        } else {
            &layout.cache
        };
        fs::create_dir_all(folder).expect("folder");
        chmod(folder, 0o777);
        match Store::open(&layout, Options::default(), &mut |_| {}).err() {
            Some(StoreError::Unsafe(path)) => assert_eq!(&path, folder),
            other => panic!("{which}: expected Unsafe, got {other:?}"),
        }
        assert_eq!(fs::read_dir(folder).expect("list").count(), 0, "{which}");
    }
}

#[test]
fn a_forged_row_cannot_make_read_allocate_before_the_frame_is_known_to_fit() {
    let scratch = Scratch::new("row_fits");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"x\ny\n"));
    drop(store);
    let range = RecordRange { first: 1, count: 2 };
    let cases = [
        (
            "an offset inside the segment header, with the length the magic reads as",
            format!(
                "UPDATE frames SET offset = 0, length = {}",
                u64::from(u32::from_le_bytes(*b"BILB")) + 4
            ),
            "the frame starts inside the segment header",
        ),
        (
            "a frame longer than the file",
            "UPDATE frames SET offset = 12, length = 1099511627776".to_string(),
            "the frame is past the end of the file",
        ),
    ];
    for (name, statement, what) in cases {
        sql(&layout, &statement);
        let store = open(&layout);
        match store.read(id, Generation(1), range) {
            Err(StoreError::Damaged { what: found, .. }) => assert_eq!(found, what, "{name}"),
            other => panic!("{name}: expected Damaged, got {other:?}"),
        }
        drop(store);
    }
}

#[test]
fn a_deleted_root_behind_a_symlinked_parent_keeps_its_binding() {
    let scratch = Scratch::new("alias_parent");
    let base = scratch.layout().root.with_file_name("base");
    let real = base.join("real");
    fs::create_dir_all(&real).expect("real");
    std::os::unix::fs::symlink(&real, base.join("alias")).expect("alias");
    let layout = Layout {
        root: base.join("alias").join("store"),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    fs::create_dir_all(real.join("store")).expect("the root exists when it is first bound");
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    fs::remove_dir_all(real.join("store")).expect("delete the root");

    let reopened = Store::open(&layout, Options::default(), &mut |_| {});
    assert!(reopened.is_ok(), "{:?}", reopened.err());
    assert!(real.join("store").is_dir());
}

#[test]
fn a_writer_that_loses_the_race_to_bind_the_state_creates_no_cache_folder() {
    let scratch = Scratch::new("writer_loses");
    let first = scratch.layout();
    open(&first).close().expect("close");
    let record = first.state.join("root");
    let bound = fs::read(&record).expect("the root record");
    fs::remove_file(&record).expect("remove");
    let second = second_root(&first);

    let holder = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(first.state.join("store.lock"))
        .expect("lock file");
    holder.lock().expect("hold the lock");
    let (waiting_tx, waiting_rx) = channel::<Notice>();
    let (done_tx, done_rx) = channel::<Option<String>>();
    let layout = second.clone();
    let writer = thread::spawn(move || {
        let result = Store::open(&layout, Options::default(), &mut |notice| {
            waiting_tx.send(notice).ok();
        });
        done_tx
            .send(result.err().map(|error| error.to_string()))
            .ok();
    });
    assert_eq!(waiting_rx.recv_timeout(WAIT), Ok(Notice::Waiting));
    fs::write(&record, bound).expect("another writer binds the state folder");
    drop(holder);
    let error = done_rx
        .recv_timeout(WAIT)
        .expect("the writer finishes")
        .expect("the writer must fail");
    assert!(error.contains("belongs to the store at"), "{error}");
    writer.join().expect("writer thread");
    assert!(!second.cache.exists());
}

fn shared_layout(scratch: &Scratch) -> (Layout, std::path::PathBuf) {
    let shared = scratch.layout().root.with_file_name("shared");
    fs::create_dir_all(&shared).expect("shared");
    let layout = Layout {
        root: shared.join("root"),
        state: shared.join("state"),
        cache: shared.join("cache"),
    };
    (layout, shared)
}

#[test]
fn an_ancestor_anyone_can_write_is_refused_before_anything_is_written() {
    let scratch = Scratch::new("open_ancestor");
    let (layout, shared) = shared_layout(&scratch);
    chmod(&shared, 0o777);
    match Store::open(&layout, Options::default(), &mut |_| {}).err() {
        Some(StoreError::Unsafe(path)) => assert_eq!(path, shared),
        other => panic!("expected Unsafe, got {other:?}"),
    }
    assert_eq!(fs::read_dir(&shared).expect("list").count(), 0);

    chmod(&shared, 0o755);
    open(&layout).close().expect("close");
    chmod(&shared, 0o777);
    let reader = Store::open_read_only(&layout, Options::default(), &mut |_| {});
    assert!(
        matches!(&reader, Err(StoreError::Unsafe(path)) if path == &shared),
        "{:?}",
        reader.err()
    );
}

#[test]
fn a_sticky_ancestor_that_anyone_can_write_is_accepted() {
    let scratch = Scratch::new("sticky_ancestor");
    let (layout, shared) = shared_layout(&scratch);
    chmod(&shared, 0o1777);
    open(&layout).close().expect("a sticky ancestor passes");
    assert!(Store::open_read_only(&layout, Options::default(), &mut |_| {}).is_ok());
}

#[test]
fn a_folder_removed_between_its_creation_and_its_check_fails_the_open() {
    let scratch = Scratch::new("vanishing_folder");
    let layout = scratch.layout();
    let root = layout.root.clone();
    let hook: Hook = Box::new(move |step| {
        if step == Step::Created {
            fs::remove_dir(&root)?;
        }
        Ok(())
    });
    let opened = Store::open(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    );
    assert!(
        matches!(&opened, Err(StoreError::Io { action: "read", path, .. }) if path == &layout.root),
        "{:?}",
        opened.err()
    );
}

#[test]
fn a_first_open_that_fails_records_no_binding_and_a_corrected_one_succeeds() {
    let scratch = Scratch::new("failed_first_open");
    let good = scratch.layout();
    let file = good.root.with_file_name("a-file");
    fs::write(&file, b"not a folder").expect("file");
    let wrong = Layout {
        root: file,
        ..good.clone()
    };
    assert!(Store::open(&wrong, Options::default(), &mut |_| {}).is_err());
    assert!(!good.state.join("root").exists());
    let corrected = Store::open(&good, Options::default(), &mut |_| {});
    assert!(corrected.is_ok(), "{:?}", corrected.err());
}

fn chown_group(path: &std::path::Path, gid: u32) {
    std::os::unix::fs::chown(path, None, Some(gid)).expect("chgrp");
}

fn effective_group() -> u32 {
    rustix::process::getegid().as_raw()
}

/// A group the user belongs to besides the effective one, if there is one.
fn another_group() -> Option<u32> {
    rustix::process::getgroups()
        .ok()?
        .into_iter()
        .map(rustix::fs::Gid::as_raw)
        .find(|&gid| gid != effective_group())
}

#[test]
fn a_folder_and_an_ancestor_that_the_effective_group_can_write_pass() {
    let scratch = Scratch::new("own_group");
    let (layout, shared) = shared_layout(&scratch);
    chown_group(&shared, effective_group());
    chmod(&shared, 0o775);
    open(&layout)
        .close()
        .expect("a group-writable ancestor of the effective group passes");
    for folder in [&layout.root, &layout.state, &layout.cache] {
        chown_group(folder, effective_group());
        chmod(folder, 0o775);
    }
    open(&layout)
        .close()
        .expect("group-writable store folders of the effective group pass");
    assert!(Store::open_read_only(&layout, Options::default(), &mut |_| {}).is_ok());
}

#[test]
fn a_folder_and_an_ancestor_that_another_group_can_write_are_refused() {
    // The Nix sandbox's build user can belong to one group only: nothing to test then.
    let Some(other) = another_group() else {
        return;
    };
    let scratch = Scratch::new("other_group");
    let (layout, shared) = shared_layout(&scratch);
    chown_group(&shared, other);
    chmod(&shared, 0o775);
    match Store::open(&layout, Options::default(), &mut |_| {}).err() {
        Some(StoreError::Unsafe(path)) => assert_eq!(path, shared),
        other => panic!("expected Unsafe for the ancestor, got {other:?}"),
    }
    assert_eq!(fs::read_dir(&shared).expect("list").count(), 0);

    chmod(&shared, 0o755);
    chown_group(&shared, effective_group());
    open(&layout).close().expect("close");
    chown_group(&layout.cache, other);
    chmod(&layout.cache, 0o775);
    match Store::open(&layout, Options::default(), &mut |_| {}).err() {
        Some(StoreError::Unsafe(path)) => assert_eq!(path, layout.cache),
        other => panic!("expected Unsafe for the folder, got {other:?}"),
    }
}

#[test]
fn a_state_path_repointed_after_its_check_does_not_redirect_the_store() {
    let scratch = Scratch::new("repointed_state");
    let base = scratch.layout().root.with_file_name("repoint");
    let real = base.join("real-state");
    let evil = base.join("evil");
    let victim = base.join("victim");
    fs::create_dir_all(&real).expect("real");
    fs::create_dir_all(&evil).expect("evil");
    fs::write(&victim, b"keep").expect("victim");
    std::os::unix::fs::symlink(&victim, evil.join("root.tmp")).expect("trap");
    let alias = base.join("alias");
    std::os::unix::fs::symlink(&real, &alias).expect("alias");
    let layout = Layout {
        root: base.join("root"),
        state: alias.clone(),
        cache: base.join("cache"),
    };
    let (swap_alias, swap_to) = (alias, evil.clone());
    let mut created = 0;
    let hook: Hook = Box::new(move |step| {
        if step == Step::Created {
            created += 1;
        }
        // The root is created first and the cache second, so the state is bound in between.
        if step == Step::Created && created == 2 {
            let _ = fs::remove_file(&swap_alias);
            let _ = std::os::unix::fs::symlink(&swap_to, &swap_alias);
        }
        Ok(())
    });
    let opened = Store::open(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    );
    assert!(opened.is_ok(), "{:?}", opened.err());
    drop(opened);
    assert_eq!(fs::read(&victim).expect("victim"), b"keep");
    assert_eq!(
        fs::read_dir(&evil).expect("list").count(),
        1,
        "only the trap"
    );
    for name in ["root", "open.seg", "store.lock"] {
        assert!(
            real.join(name).exists(),
            "{name} belongs in the checked state folder"
        );
    }
}

#[test]
fn a_missing_parent_made_writable_before_the_folder_is_created_is_refused_before_a_store_file_is_written()
 {
    let scratch = Scratch::new("raced_parent");
    let base = scratch.layout().root.with_file_name("raced");
    fs::create_dir_all(&base).expect("base");
    let parent = base.join("newparent");
    let layout = Layout {
        root: parent.join("root"),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    let planted = parent.clone();
    let hook: Hook = Box::new(move |step| {
        if step == Step::Creating && !planted.exists() {
            fs::create_dir(&planted)?;
            chmod(&planted, 0o777);
        }
        Ok(())
    });
    let opened = Store::open(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    );
    assert!(
        matches!(&opened, Err(StoreError::Unsafe(path)) if path == &parent),
        "{:?}",
        opened.err()
    );
    for name in ["root", "open.seg"] {
        assert!(
            !layout.state.join(name).exists(),
            "{name} must not be written"
        );
    }
}

#[test]
fn a_reader_of_a_missing_state_folder_uses_its_resolved_path_too() {
    let scratch = Scratch::new("repointed_missing_state");
    let base = scratch.layout().root.with_file_name("reader-repoint");
    let first = Layout {
        root: base.join("root"),
        state: base.join("elsewhere").join("state"),
        cache: base.join("cache"),
    };
    let mut store = open(&first);
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    store.close().expect("close");
    let evil = base.join("evil");
    fs::create_dir_all(&evil).expect("evil");
    fs::rename(first.state.parent().expect("parent"), base.join("moved")).expect("move");
    fs::create_dir_all(base.join("real")).expect("real");
    let alias = base.join("alias");
    std::os::unix::fs::symlink(base.join("real"), &alias).expect("alias");
    fs::create_dir_all(evil.join("state")).expect("a state folder in the trap");
    fs::write(evil.join("state").join("root"), b"/nowhere\n").expect("a binding in the trap");
    fs::write(evil.join("state").join("store.lock"), b"").expect("a lock file in the trap");
    let layout = Layout {
        state: alias.join("state"),
        ..first
    };
    let (swap_alias, swap_to) = (alias, evil);
    let hook: Hook = Box::new(move |step| {
        if step == Step::Checked {
            let _ = fs::remove_file(&swap_alias);
            let _ = std::os::unix::fs::symlink(&swap_to, &swap_alias);
        }
        Ok(())
    });
    let reader = Store::open_read_only(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    );
    assert!(
        reader.is_ok(),
        "the reader must not read the trap's binding: {:?}",
        reader.err()
    );
    assert_eq!(read_records(&reader.expect("reader"), a, 1, 1), b"one\n");
}

#[test]
fn a_root_that_holds_0_20s_notes_or_library_is_refused_before_anything_is_written() {
    for name in ["notes", "library"] {
        let scratch = Scratch::new("older_root");
        let layout = scratch.layout();
        fs::create_dir_all(layout.root.join(name)).expect("a 0.20 folder");
        let before = snapshot_of(&layout);
        let expected = format!("{} belongs to bilbo 0.20", layout.root.join(name).display());
        let writer = Store::open(&layout, Options::default(), &mut |_| {});
        assert_eq!(
            writer.err().map(|e| e.to_string()),
            Some(expected.clone()),
            "{name}"
        );
        let reader = Store::open_read_only(&layout, Options::default(), &mut |_| {});
        assert_eq!(
            reader.err().map(|e| e.to_string()),
            Some(expected),
            "{name}"
        );
        let check = Store::check(&layout).expect_err("refused");
        assert!(
            matches!(&check, StoreError::Older(folder) if folder == &layout.root.join(name)),
            "{name}: {check}"
        );
        assert_eq!(snapshot_of(&layout), before, "{name}");
        assert!(!layout.state.exists() && !layout.cache.exists(), "{name}");
    }
}

#[test]
fn a_root_that_cannot_be_read_is_an_error_not_an_absence_and_nothing_is_written() {
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            chmod(&self.0, 0o700);
        }
    }

    let scratch = Scratch::new("unreadable_root");
    let layout = scratch.layout();
    fs::create_dir_all(&layout.root).expect("root");
    chmod(&layout.root, 0o000);
    let _restore = Restore(layout.root.clone());
    if fs::read_dir(&layout.root).is_ok() {
        return;
    }
    let writer = Store::open(&layout, Options::default(), &mut |_| {});
    match writer.err() {
        Some(StoreError::Io { path, .. }) => assert_eq!(path, layout.root.join("notes")),
        other => panic!("expected an io error naming notes, got {other:?}"),
    }
    let reader = Store::open_read_only(&layout, Options::default(), &mut |_| {});
    assert!(matches!(reader.err(), Some(StoreError::Io { .. })));
    assert!(!layout.state.exists() && !layout.cache.exists());
}

fn repoint_at_the_guard(link: &std::path::Path, to: &std::path::Path) -> Options {
    let (link, to) = (link.to_path_buf(), to.to_path_buf());
    let hook: Hook = Box::new(move |step| {
        if step == Step::Guarded {
            let _ = fs::remove_file(&link);
            let _ = std::os::unix::fs::symlink(&to, &link);
        }
        Ok(())
    });
    Options {
        hook: Some(hook),
        ..Options::default()
    }
}

#[test]
fn a_root_link_repointed_after_the_guard_never_opens_a_0_20_root_for_writing() {
    let scratch = Scratch::new("repointed_older_root");
    let base = scratch.layout().root.with_file_name("repoint-older");
    let clean = base.join("clean");
    let older = base.join("older");
    fs::create_dir_all(&clean).expect("clean");
    fs::create_dir_all(older.join("notes")).expect("a 0.20 folder");
    let link = base.join("link");
    std::os::unix::fs::symlink(&clean, &link).expect("link");
    let layout = Layout {
        root: link.clone(),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    let opened = Store::open(&layout, repoint_at_the_guard(&link, &older), &mut |_| {});
    assert!(opened.is_ok(), "{:?}", opened.err());
    drop(opened);
    assert_eq!(
        snapshot(&older).keys().collect::<Vec<_>>(),
        [&older.join("notes")],
        "nothing is written into the 0.20 root"
    );
    let mut bound = clean.as_os_str().as_bytes().to_vec();
    bound.push(b'\n');
    assert_eq!(
        String::from_utf8_lossy(&fs::read(layout.state.join("root")).expect("binding")),
        String::from_utf8_lossy(&bound)
    );
}

#[test]
fn a_root_link_repointed_after_the_guard_never_lets_a_reader_read_a_0_20_root() {
    let scratch = Scratch::new("repointed_older_reader");
    let base = scratch.layout().root.with_file_name("repoint-older-reader");
    let clean = base.join("clean");
    let older = base.join("older");
    fs::create_dir_all(&clean).expect("clean");
    let mut store = open(&Layout {
        root: older.clone(),
        state: base.join("older-state"),
        cache: base.join("older-cache"),
    });
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    fs::create_dir_all(older.join("notes")).expect("a 0.20 folder");
    let link = base.join("link");
    std::os::unix::fs::symlink(&clean, &link).expect("link");
    let layout = Layout {
        root: link.clone(),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    let reader = Store::open_read_only(&layout, repoint_at_the_guard(&link, &older), &mut |_| {})
        .expect("reader");
    assert!(
        reader.by_origin(&origin("a.md")).expect("lookup").is_none(),
        "the reader must read the checked folder"
    );
}

fn nothing_was_bound_or_written(layout: &Layout, older: &std::path::Path) {
    for name in ["root", "open.seg"] {
        assert!(
            !layout.state.join(name).exists(),
            "{name} must not be written"
        );
    }
    assert_eq!(
        snapshot(older).keys().collect::<Vec<_>>(),
        [&older.join("notes")],
        "nothing is written into the 0.20 root"
    );
}

#[test]
fn a_missing_root_made_a_link_to_a_0_20_root_after_the_guard_is_refused_before_a_binding_is_written()
 {
    let scratch = Scratch::new("missing_root_linked");
    let base = scratch.layout().root.with_file_name("missing-linked");
    let older = base.join("older");
    fs::create_dir_all(older.join("notes")).expect("a 0.20 folder");
    let before = snapshot(&older);
    let layout = Layout {
        root: base.join("root"),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    let opened = Store::open(
        &layout,
        repoint_at_the_guard(&layout.root, &older),
        &mut |_| {},
    );
    assert!(
        matches!(&opened, Err(StoreError::Older(folder)) if folder == &layout.root.join("notes")),
        "{:?}",
        opened.err()
    );
    nothing_was_bound_or_written(&layout, &older);
    assert_eq!(snapshot(&older), before);
}

#[test]
fn a_dangling_root_link_repointed_at_a_0_20_root_after_the_guard_is_refused_before_a_binding_is_written()
 {
    let scratch = Scratch::new("dangling_root_repointed");
    let base = scratch.layout().root.with_file_name("dangling-repointed");
    let older = base.join("older");
    fs::create_dir_all(older.join("notes")).expect("a 0.20 folder");
    let before = snapshot(&older);
    let link = base.join("link");
    std::os::unix::fs::symlink(base.join("gone"), &link).expect("a dangling link");
    let layout = Layout {
        root: link.clone(),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    let opened = Store::open(&layout, repoint_at_the_guard(&link, &older), &mut |_| {});
    assert!(
        matches!(&opened, Err(StoreError::Older(folder)) if folder == &link.join("notes")),
        "{:?}",
        opened.err()
    );
    nothing_was_bound_or_written(&layout, &older);
    assert_eq!(snapshot(&older), before);
}

#[test]
fn a_root_replaced_by_a_link_to_a_0_20_root_after_the_guard_is_refused_to_a_reader() {
    let scratch = Scratch::new("reader_root_replaced");
    let base = scratch.layout().root.with_file_name("reader-replaced");
    let older = base.join("older");
    fs::create_dir_all(older.join("notes")).expect("a 0.20 folder");
    let layout = Layout {
        root: base.join("root"),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    fs::create_dir_all(&layout.root).expect("root");
    let (root, to) = (layout.root.clone(), older.clone());
    let hook: Hook = Box::new(move |step| {
        if step == Step::Guarded {
            fs::remove_dir(&root)?;
            std::os::unix::fs::symlink(&to, &root)?;
        }
        Ok(())
    });
    let reader = Store::open_read_only(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    );
    assert!(
        matches!(&reader, Err(StoreError::Older(folder)) if folder == &layout.root.join("notes")),
        "{:?}",
        reader.err()
    );
    nothing_was_bound_or_written(&layout, &older);
}

#[test]
fn a_root_replaced_by_a_link_to_another_clean_store_after_the_guard_is_refused_to_a_reader() {
    let scratch = Scratch::new("reader_root_swapped");
    let base = scratch.layout().root.with_file_name("reader-swapped");
    let layout = Layout {
        root: base.join("a"),
        state: base.join("a-state"),
        cache: base.join("a-cache"),
    };
    let mut store = open(&layout);
    keep(&mut store, "a.md", b"one\n");
    store.close().expect("close");
    let other = Layout {
        root: base.join("b"),
        state: base.join("b-state"),
        cache: base.join("b-cache"),
    };
    let mut store = open(&other);
    keep(&mut store, "b.md", b"two\n");
    store.close().expect("close");
    let (root, moved, to) = (
        layout.root.clone(),
        base.join("a-moved"),
        other.root.clone(),
    );
    let hook: Hook = Box::new(move |step| {
        if step == Step::Guarded {
            fs::rename(&root, &moved)?;
            std::os::unix::fs::symlink(&to, &root)?;
        }
        Ok(())
    });
    let reader = Store::open_read_only(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    );
    assert!(
        matches!(&reader, Err(StoreError::OtherRoot { .. })),
        "{:?}",
        reader.err()
    );
}

#[test]
fn a_root_that_vanishes_after_the_guard_is_missing_to_a_reader_not_an_io_error() {
    let scratch = Scratch::new("reader_root_vanishes");
    let layout = scratch.layout();
    fs::create_dir_all(&layout.root).expect("root");
    let root = layout.root.clone();
    let hook: Hook = Box::new(move |step| {
        if step == Step::Guarded {
            fs::remove_dir(&root)?;
        }
        Ok(())
    });
    let reader = Store::open_read_only(
        &layout,
        Options {
            hook: Some(hook),
            ..Options::default()
        },
        &mut |_| {},
    );
    assert!(
        matches!(&reader, Err(StoreError::Missing(text)) if text.contains(&*layout.root.to_string_lossy())),
        "{:?}",
        reader.err()
    );
}

#[test]
fn a_check_passes_a_clean_or_missing_root_and_creates_nothing() {
    let scratch = Scratch::new("check_clean");
    let layout = scratch.layout();
    assert!(Store::check(&layout).is_ok());
    assert!(!layout.root.exists() && !layout.state.exists() && !layout.cache.exists());
    fs::create_dir_all(&layout.root).expect("root");
    assert!(Store::check(&layout).is_ok());
}

#[test]
fn a_check_follows_a_root_link_to_the_folder_it_points_at() {
    let scratch = Scratch::new("check_link");
    let base = scratch.layout().root.with_file_name("check-link");
    let older = base.join("older");
    fs::create_dir_all(older.join("notes")).expect("a 0.20 folder");
    let link = base.join("link");
    std::os::unix::fs::symlink(&older, &link).expect("link");
    let layout = Layout {
        root: link.clone(),
        state: base.join("state"),
        cache: base.join("cache"),
    };
    assert!(matches!(
        Store::check(&layout),
        Err(StoreError::Older(folder)) if folder == link.join("notes")
    ));
}

fn refused_inside(result: Result<(), StoreError>, folder: &str, what: &str) {
    match result {
        Err(StoreError::Inside { folder: found, .. }) => assert_eq!(found, folder, "{what}"),
        other => panic!("{what}: {other:?}"),
    }
}

fn inside_layouts(base: &std::path::Path) -> Vec<(&'static str, &'static str, Layout)> {
    let case = |name: &str| base.join(name);
    let mut layouts = Vec::new();
    let root = case("state-notes").join("root");
    layouts.push((
        "state",
        "state at notes",
        Layout {
            state: root.join("notes"),
            cache: case("state-notes").join("cache"),
            root,
        },
    ));
    let root = case("cache-library").join("root");
    layouts.push((
        "cache",
        "cache beneath library",
        Layout {
            cache: root.join("library").join("x"),
            state: case("cache-library").join("state"),
            root,
        },
    ));
    let root = case("cache-plain").join("root");
    layouts.push((
        "cache",
        "cache at cache",
        Layout {
            cache: root.join("cache"),
            state: case("cache-plain").join("state"),
            root,
        },
    ));
    let root = case("state-root").join("root");
    layouts.push((
        "state",
        "state equal to the root",
        Layout {
            state: root.clone(),
            cache: case("state-root").join("cache"),
            root,
        },
    ));
    let root = case("dot-dot").join("root");
    layouts.push((
        "state",
        "state through a dot-dot",
        Layout {
            state: root.join("gone").join("..").join("notes"),
            cache: case("dot-dot").join("cache"),
            root,
        },
    ));
    layouts
}

#[test]
fn a_state_or_cache_folder_inside_a_missing_root_is_refused_and_nothing_is_created() {
    let scratch = Scratch::new("inside_missing_root");
    let base = scratch.layout().root.with_file_name("inside-missing");
    for (folder, name, layout) in inside_layouts(&base) {
        let refused = |result: Result<(), StoreError>, how: &str| {
            refused_inside(result, folder, &format!("{name}: {how}"));
        };
        refused(Store::check(&layout), "check");
        refused(
            Store::open(&layout, Options::default(), &mut |_| {}).map(|_| ()),
            "open",
        );
        refused(
            Store::open_read_only(&layout, Options::default(), &mut |_| {}).map(|_| ()),
            "open_read_only",
        );
        assert!(
            !layout.root.exists() && !layout.state.exists() && !layout.cache.exists(),
            "{name}: nothing was created"
        );
    }
}

#[test]
fn a_state_or_cache_folder_inside_an_existing_root_is_refused_by_all_three_and_the_root_gains_nothing()
 {
    let scratch = Scratch::new("inside_present_root");
    let base = scratch.layout().root.with_file_name("inside-present");
    for (folder, name, layout) in inside_layouts(&base) {
        fs::create_dir_all(&layout.root).expect("root");
        let before = snapshot(&base);
        let refused = |result: Result<(), StoreError>, how: &str| {
            refused_inside(result, folder, &format!("{name}: {how}"));
        };
        refused(Store::check(&layout), "check");
        refused(
            Store::open(&layout, Options::default(), &mut |_| {}).map(|_| ()),
            "open",
        );
        refused(
            Store::open_read_only(&layout, Options::default(), &mut |_| {}).map(|_| ()),
            "open_read_only",
        );
        assert_eq!(snapshot(&base), before, "{name}: nothing was created");
    }
}

#[test]
fn a_state_or_cache_folder_inside_the_root_through_a_link_is_refused() {
    let scratch = Scratch::new("inside_through_link");
    let base = scratch.layout().root.with_file_name("inside-link");
    let real = base.join("real");
    let link = base.join("link");
    fs::create_dir_all(&real).expect("real root");
    std::os::unix::fs::symlink(&real, &link).expect("a link to the root");
    for (name, layout) in [
        (
            "root by link, state by path",
            Layout {
                root: link.clone(),
                state: real.join("notes"),
                cache: base.join("cache"),
            },
        ),
        (
            "root by path, cache by link",
            Layout {
                root: real.clone(),
                state: base.join("state"),
                cache: link.join("cache"),
            },
        ),
    ] {
        let before = snapshot(&base);
        assert!(
            matches!(Store::check(&layout), Err(StoreError::Inside { .. })),
            "{name}: check"
        );
        assert!(
            matches!(
                Store::open(&layout, Options::default(), &mut |_| {}),
                Err(StoreError::Inside { .. })
            ),
            "{name}: open"
        );
        assert!(
            matches!(
                Store::open_read_only(&layout, Options::default(), &mut |_| {}),
                Err(StoreError::Inside { .. })
            ),
            "{name}: open_read_only"
        );
        assert_eq!(snapshot(&base), before, "{name}: nothing was created");
    }
}

#[test]
fn an_inside_error_names_the_folder_and_the_root() {
    let error = StoreError::Inside {
        folder: "cache",
        path: "/r/cache".into(),
        root: "/r".into(),
    };
    assert_eq!(
        error.to_string(),
        "the cache folder /r/cache is inside the store root /r, which holds only segments"
    );
}

/// Every entry under `dir`, links not followed, so a dangling link is listed and not read.
fn listing(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(next) = todo.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("entry").path();
            if fs::symlink_metadata(&path).expect("meta").is_dir() {
                todo.push(path.clone());
            }
            found.push(path);
        }
    }
    found.sort();
    found
}

fn link(to: &std::path::Path, at: &std::path::Path) {
    std::os::unix::fs::symlink(to, at).expect("link");
}

fn refused_by_all_three(layout: &Layout, folder: &str, what: &str) {
    refused_inside(Store::check(layout), folder, &format!("{what}: check"));
    refused_inside(
        Store::open(layout, Options::default(), &mut |_| {}).map(|_| ()),
        folder,
        &format!("{what}: open"),
    );
    refused_inside(
        Store::open_read_only(layout, Options::default(), &mut |_| {}).map(|_| ()),
        folder,
        &format!("{what}: open_read_only"),
    );
}

#[test]
fn a_state_or_cache_link_dangling_into_a_missing_root_is_refused_and_nothing_is_created() {
    let scratch = Scratch::new("inside_dangling_link");
    let base = scratch.layout().root.with_file_name("inside-dangling");
    fs::create_dir_all(&base).expect("base");
    let root = base.join("root");
    let hop = base.join("hop");
    let nested = base.join("nested");
    fs::create_dir_all(&nested).expect("nested");
    link(&root, &hop);
    let cases = [
        (
            "state",
            "state link to the root",
            Layout {
                root: root.clone(),
                state: hop.clone(),
                cache: base.join("cache"),
            },
        ),
        (
            "cache",
            "cache link to the root",
            Layout {
                root: root.clone(),
                state: base.join("state"),
                cache: hop.clone(),
            },
        ),
        (
            "state",
            "state link to a folder beneath the root",
            Layout {
                root: root.clone(),
                state: {
                    link(&root.join("notes"), &nested.join("deep"));
                    nested.join("deep")
                },
                cache: base.join("cache"),
            },
        ),
        (
            "cache",
            "cache chain of two links, the last relative",
            Layout {
                root: root.clone(),
                state: base.join("state"),
                cache: {
                    link(std::path::Path::new("../hop"), &nested.join("via"));
                    nested.join("via")
                },
            },
        ),
        (
            "state",
            "state link with a dot-dot tail",
            Layout {
                root: root.clone(),
                state: hop.join("gone").join("..").join("x"),
                cache: base.join("cache"),
            },
        ),
    ];
    let before = listing(&base);
    for (folder, name, layout) in cases {
        refused_by_all_three(&layout, folder, name);
        assert!(!root.exists(), "{name}: the root stays missing");
        assert_eq!(listing(&base), before, "{name}: nothing was created");
    }
}

#[test]
fn a_state_or_cache_link_to_a_sibling_folder_passes_the_check_whether_it_exists_or_dangles() {
    let scratch = Scratch::new("inside_sibling_link");
    let base = scratch.layout().root.with_file_name("inside-sibling");
    fs::create_dir_all(&base).expect("base");
    let root = base.join("root");
    let present = base.join("present");
    fs::create_dir_all(&present).expect("present");
    let to_present = base.join("to-present");
    let to_absent = base.join("to-absent");
    link(&present, &to_present);
    link(&base.join("absent"), &to_absent);
    let before = listing(&base);
    for (name, state, cache) in [
        ("state to an existing sibling", &to_present, &to_absent),
        ("cache to an existing sibling", &to_absent, &to_present),
    ] {
        let layout = Layout {
            root: root.clone(),
            state: state.clone(),
            cache: cache.clone(),
        };
        assert!(Store::check(&layout).is_ok(), "{name}");
        assert_eq!(listing(&base), before, "{name}: nothing was created");
    }
    let layout = Layout {
        root: root.clone(),
        state: to_present,
        cache: base.join("cache"),
    };
    Store::open(&layout, Options::default(), &mut |_| {}).expect("a writer opens it");
    assert!(
        Store::open_read_only(&layout, Options::default(), &mut |_| {}).is_ok(),
        "a reader opens it"
    );
}

#[test]
fn a_state_link_loop_is_an_error_and_not_a_hang() {
    let scratch = Scratch::new("inside_link_loop");
    let base = scratch.layout().root.with_file_name("inside-loop");
    fs::create_dir_all(&base).expect("base");
    let one = base.join("one");
    let two = base.join("two");
    link(&two, &one);
    link(&one, &two);
    let layout = Layout {
        root: base.join("root"),
        state: one,
        cache: base.join("cache"),
    };
    assert!(matches!(Store::check(&layout), Err(StoreError::Io { .. })));
    assert!(!layout.root.exists());
}

#[test]
fn a_state_folder_beneath_the_target_of_a_dangling_root_link_is_refused() {
    let scratch = Scratch::new("inside_dangling_root");
    let base = scratch.layout().root.with_file_name("inside-dangling-root");
    fs::create_dir_all(&base).expect("base");
    let target = base.join("target");
    let root = base.join("root");
    link(&target, &root);
    let layout = Layout {
        root,
        state: target.join("notes"),
        cache: base.join("cache"),
    };
    let before = listing(&base);
    refused_by_all_three(&layout, "state", "state beneath the root link's target");
    assert!(!target.exists());
    assert_eq!(listing(&base), before);
}
