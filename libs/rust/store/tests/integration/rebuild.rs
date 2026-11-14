use std::fs;
use std::time::{Duration, UNIX_EPOCH};

use bilbo_store::entries::{Capture, Captured, Cursor, Entries, Head, Stamp};
use bilbo_store::id::Id;
use bilbo_store::labels::{Labels, Relabel};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Cache, Notice, Options, Rebuild, Step, Store};

use crate::scratch::{
    Scratch, capture_label, catalog, ends, filed, label_rows, open, open_with, read_records,
    sealing, words,
};
use crate::seal::stopping_at;

fn stamp(seconds: u64) -> Stamp {
    Stamp {
        size: seconds,
        modified: UNIX_EPOCH + Duration::from_secs(1_700_000_000 + seconds),
    }
}

fn capture(store: &mut Store, name: &str, bytes: &[u8], stamp: Option<Stamp>) {
    let label = capture_label(name);
    store
        .capture(&Capture {
            label: &label,
            kind: "markdown",
            id: None,
            id_label: None,
            stamp,
            bytes,
            ends: &ends(bytes),
        })
        .expect("capture");
}

fn head(store: &Store, name: &str) -> Head {
    store
        .by_capture(&capture_label(name))
        .expect("a head")
        .expect("an entry")
}

fn anew(layout: &Layout) -> (Store, Vec<Notice>) {
    let mut notices = Vec::new();
    let options = Options {
        rebuild: Rebuild::Always,
        ..sealing()
    };
    let store = Store::open(layout, options, &mut |notice| notices.push(notice)).expect("open");
    (store, notices)
}

fn catalog_id(store: &Store) -> [u8; 16] {
    store
        .changes(&Cursor::start())
        .expect("changes")
        .1
        .catalog()
}

#[test]
fn a_rebuild_that_is_asked_for_makes_a_new_catalog_of_the_same_entries() {
    let scratch = Scratch::new("anew");
    let layout = scratch.layout();
    let mut store = open(&layout);
    capture(&mut store, "a.md", b"one\ntwo\n", Some(stamp(1)));
    capture(&mut store, "b.md", b"three\n", None);
    store.close().expect("close");
    let mut store = open(&layout);
    capture(&mut store, "a.md", b"one\ntwo\nmore\n", Some(stamp(2)));
    let before = (head(&store, "a.md"), head(&store, "b.md"));
    let (changes, cursor) = store.changes(&Cursor::start()).expect("changes");
    store.release();

    let (store, notices) = anew(&layout);
    assert_eq!(
        notices,
        [Notice::RebuiltCatalog {
            entries: 2,
            segments: 1
        }]
    );
    assert_ne!(
        catalog_id(&store),
        cursor.catalog(),
        "a rebuilt catalog is another catalog"
    );
    assert_eq!((head(&store, "a.md"), head(&store, "b.md")), before);
    assert_eq!(read_records(&store, before.0.id, 1, 3), b"one\ntwo\nmore\n");
    let (again, _) = store.changes(&Cursor::start()).expect("changes");
    assert_eq!(again.len(), changes.len());
    store.release();

    let (store, _) = anew(&layout);
    assert_eq!((head(&store, "a.md"), head(&store, "b.md")), before);
}

#[test]
fn a_stamp_comes_back_only_for_a_head_that_comes_back_the_same() {
    let scratch = Scratch::new("anew-stamps");
    let layout = scratch.layout();
    let mut store = open(&layout);
    capture(&mut store, "kept.md", b"one\n", Some(stamp(1)));
    capture(&mut store, "grown.md", b"one\n", Some(stamp(2)));
    capture(&mut store, "bare.md", b"one\n", None);
    drop(store);
    // The frame is in open.seg and the catalog has not heard of it: its stamp is one capture old.
    let behind = Options {
        hook: Some(stopping_at(Step::Appended)),
        ..sealing()
    };
    let mut store = open_with(&layout, behind);
    let label = capture_label("grown.md");
    let cut = store.capture(&Capture {
        label: &label,
        kind: "markdown",
        id: None,
        id_label: None,
        stamp: Some(stamp(3)),
        bytes: b"one\ntwo\n",
        ends: &ends(b"one\ntwo\n"),
    });
    assert!(cut.is_err());
    drop(store);

    let (store, _) = anew(&layout);
    assert_eq!(head(&store, "kept.md").stamp, Some(stamp(1)));
    assert_eq!(head(&store, "grown.md").records, 2);
    assert_eq!(
        head(&store, "grown.md").stamp,
        None,
        "a stamp of an older head is not kept"
    );
    assert_eq!(head(&store, "bare.md").stamp, None);
}

#[test]
fn a_stamp_is_not_carried_to_a_head_whose_mark_alone_differs() {
    let scratch = Scratch::new("anew-mark-differs");
    let layout = scratch.layout();
    let mut store = open(&layout);
    capture(&mut store, "same.md", b"one\n", Some(stamp(1)));
    capture(&mut store, "marked.md", b"two\n", Some(stamp(2)));
    let marked = head(&store, "marked.md");
    store.close().expect("close");
    crate::scratch::sql(
        &layout,
        &format!(
            "UPDATE generations SET origin_hash = zeroblob(16) WHERE entry = x'{}'",
            marked
                .id
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    );

    let (store, _) = anew(&layout);
    let rebuilt = head(&store, "marked.md");
    assert_eq!(rebuilt.mark, marked.mark, "the segments hold the real mark");
    assert_eq!(
        (rebuilt.generation, rebuilt.records),
        (marked.generation, marked.records)
    );
    assert_eq!(rebuilt.stamp, None, "another mark is another head");
    assert_eq!(head(&store, "same.md").stamp, Some(stamp(1)));
}

#[test]
fn a_stamp_is_not_carried_to_an_entry_whose_capture_label_changed_under_the_old_catalog() {
    let scratch = Scratch::new("anew-capture-label-moved");
    let layout = scratch.layout();
    let mut store = open(&layout);
    capture(&mut store, "a.md", b"one\n", Some(stamp(1)));
    let id = head(&store, "a.md").id;
    let mut meta = store.meta(id).expect("meta").expect("some");
    drop(store);
    meta.labels.capture = Some(capture_label("b.md"));
    let behind = Options {
        hook: Some(stopping_at(Step::Appended)),
        ..sealing()
    };
    let mut store = open_with(&layout, behind);
    assert!(store.set_meta(id, &meta).is_err());
    drop(store);

    let (mut store, _) = anew(&layout);
    let moved = head(&store, "b.md");
    assert_eq!(moved.id, id);
    assert_eq!(moved.stamp, None);
    let label = capture_label("b.md");
    let again = store
        .capture(&Capture {
            label: &label,
            kind: "markdown",
            id: None,
            id_label: None,
            stamp: Some(stamp(1)),
            bytes: b"two\n",
            ends: &ends(b"two\n"),
        })
        .expect("capture");
    assert!(!matches!(again, Captured::Unchanged(_)));
}

#[test]
fn a_meta_update_that_changes_the_capture_label_drops_the_stamp_and_one_that_keeps_it_does_not() {
    let scratch = Scratch::new("meta-stamp");
    let layout = scratch.layout();
    let mut store = open(&layout);
    capture(&mut store, "a.md", b"one\n", Some(stamp(1)));
    capture(&mut store, "c.md", b"one\n", Some(stamp(2)));
    let a = head(&store, "a.md").id;
    let c = head(&store, "c.md").id;

    let mut meta = store.meta(c).expect("meta").expect("some");
    meta.labels.id = Some(Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DF").expect("id"));
    store.set_meta(c, &meta).expect("set_meta");
    assert_eq!(head(&store, "c.md").stamp, Some(stamp(2)));

    let mut meta = store.meta(a).expect("meta").expect("some");
    meta.labels.capture = Some(capture_label("new.md"));
    store.set_meta(a, &meta).expect("set_meta");
    assert_eq!(head(&store, "new.md").stamp, None);
}

type Break = (&'static str, fn(&Layout));

#[test]
fn the_stamps_are_lost_with_a_catalog_that_is_missing_unreadable_or_another_stores() {
    let breaks: [Break; 3] = [
        ("missing", |layout| {
            fs::remove_file(catalog(layout)).expect("remove");
        }),
        ("unreadable", |layout| {
            fs::write(catalog(layout), b"not a database").expect("write");
        }),
        ("another store's", |layout| {
            crate::scratch::sql(
                layout,
                "UPDATE catalog SET value = x'2f6f74686572' WHERE key = 'root'",
            );
        }),
    ];
    for (what, break_it) in breaks {
        let scratch = Scratch::new("anew-lost");
        let layout = scratch.layout();
        let mut store = open(&layout);
        capture(&mut store, "a.md", b"one\n", Some(stamp(1)));
        let before = head(&store, "a.md");
        store.close().expect("close");
        break_it(&layout);
        let (store, _) = anew(&layout);
        let after = head(&store, "a.md");
        assert_eq!(after.stamp, None, "{what}");
        assert_eq!(
            (after.generation, after.records, after.mark),
            (before.generation, before.records, before.mark),
            "{what}"
        );
    }
}

#[test]
fn an_empty_store_gets_its_catalog_made_without_a_word() {
    let scratch = Scratch::new("anew-empty");
    let (store, notices) = anew(&scratch.layout());
    assert_eq!(notices, []);
    assert_eq!(store.changes(&Cursor::start()).expect("changes").0, []);
    assert!(catalog(&scratch.layout()).exists());
}

#[test]
fn a_catalog_made_again_has_no_text_label_and_a_new_id() {
    let scratch = Scratch::new("anew-no-text-labels");
    let layout = scratch.layout();
    let mut store = open(&layout);
    capture(&mut store, "agent.jsonl", b"{}\n", None);
    capture(&mut store, "session.jsonl", b"{}\n", None);
    let agent = head(&store, "agent.jsonl").id;
    let session = head(&store, "session.jsonl").id;
    let mut meta = store.meta(agent).expect("meta").expect("some");
    meta.labels.id = Some(session);
    store.set_meta(agent, &meta).expect("set_meta");
    store
        .relabel(&[filed(agent, &["home me shed"])])
        .expect("relabel");
    let before = catalog_id(&store);
    store.close().expect("close");
    let (store, _) = anew(&layout);
    assert_ne!(catalog_id(&store), before);
    assert_eq!(store.with_words(&words("shed")).expect("find"), []);
    assert_eq!(label_rows(&layout), (0, 0, 0));
    assert_eq!(head(&store, "agent.jsonl").id, agent);
    assert_eq!(store.belonging_to(&[session]).expect("found"), [agent]);
    assert_eq!(store.meta(agent).expect("meta").expect("some"), meta);
}

#[test]
fn a_catalog_of_an_older_version_is_made_again_with_the_label_tables() {
    for older in [2, 3, 4] {
        let scratch = Scratch::new("anew-older-version");
        let layout = scratch.layout();
        let mut store = open(&layout);
        capture(&mut store, "a.md", b"one\n", None);
        let a = head(&store, "a.md").id;
        store.relabel(&[filed(a, &["shed"])]).expect("relabel");
        let before = catalog_id(&store);
        store.close().expect("close");
        crate::scratch::sql(&layout, &format!("PRAGMA user_version = {older}"));
        let store = open(&layout);
        assert_ne!(catalog_id(&store), before, "{older}");
        assert_eq!(head(&store, "a.md").id, a);
        assert_eq!(label_rows(&layout), (0, 0, 0), "{older}");
        drop(store);
        let version: i64 = rusqlite::Connection::open(catalog(&layout))
            .expect("open")
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, 5);
    }
}

#[test]
fn a_reader_finds_no_kept_catalog_in_a_file_of_an_older_version() {
    let scratch = Scratch::new("reader-older-version");
    let layout = scratch.layout();
    let mut store = open(&layout);
    capture(&mut store, "a.md", b"one\n", None);
    let a = head(&store, "a.md").id;
    store.relabel(&[filed(a, &["shed"])]).expect("relabel");
    store.close().expect("close");
    crate::scratch::sql(&layout, "PRAGMA user_version = 4");
    let held = fs::read(catalog(&layout)).expect("read");
    let canonical = fs::canonicalize(&layout.cache).expect("canonical");
    let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only open");
    assert_eq!(store.reader_cache(), &Cache::NoCatalog(canonical));
    assert_eq!(read_records(&store, a, 1, 1), b"one\n");
    assert_eq!(store.with_words(&words("shed")).expect("find"), []);
    drop(store);
    assert_eq!(fs::read(catalog(&layout)).expect("read"), held);
}
