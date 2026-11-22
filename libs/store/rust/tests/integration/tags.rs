use std::fs;

use bilbo_store::entries::{Change, Cursor, Entries, Generation, Meta, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::labels::{By, Labels, Standing, Tag, Tagged, Tags};
use bilbo_store::segments::Store;

use crate::scratch::{
    Scratch, catalog, id_of, keep, keep_as, keep_tagged, open, open_seg, sealing, snapshot_of,
};

fn tag(name: &str) -> Tag {
    Tag::new(name).expect("tag")
}

fn tags_of(standings: &[(&str, Standing)]) -> Tags {
    let mut tags = Tags::default();
    for (name, standing) in standings {
        tags.set(tag(name), Some(*standing));
    }
    tags
}

const PERSON: Standing = Standing::Carried(By::Person);
const MODEL: Standing = Standing::Carried(By::Model);

fn tags_of_entry(store: &Store, id: Id) -> Tags {
    store.meta(id).expect("meta").expect("some").labels.tags
}

#[test]
fn a_tag_given_at_first_capture_is_in_the_meta_after_a_reopen_and_after_a_rebuild() {
    let scratch = Scratch::new("tag_first_capture");
    let layout = scratch.layout();
    let given = tags_of(&[("a", PERSON)]);
    let mut store = open(&layout);
    let id = id_of(&keep_tagged(
        &mut store, "a.md", "markdown", b"one\n", &given,
    ));
    assert_eq!(tags_of_entry(&store, id), given);
    drop(store);
    let store = open(&layout);
    assert_eq!(tags_of_entry(&store, id), given);
    drop(store);
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let store = open(&layout);
    assert_eq!(tags_of_entry(&store, id), given);
}

#[test]
fn a_later_capture_does_not_touch_the_tags() {
    let scratch = Scratch::new("tag_later_capture");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep_tagged(
        &mut store,
        "a.md",
        "markdown",
        b"one\n",
        &tags_of(&[("a", PERSON)]),
    ));
    let mut meta = store.meta(id).expect("meta").expect("some");
    meta.labels.tags.set(tag("b"), Some(MODEL));
    store.set_meta(id, &meta).expect("set_meta");
    let both = tags_of(&[("a", PERSON), ("b", MODEL)]);
    assert_eq!(tags_of_entry(&store, id), both);
    keep(&mut store, "a.md", b"one\ntwo\n");
    assert_eq!(tags_of_entry(&store, id), both);
    keep(&mut store, "a.md", b"other\n");
    assert_eq!(tags_of_entry(&store, id), both);
    drop(store);
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    let store = open(&layout);
    assert_eq!(tags_of_entry(&store, id), both);
}

fn fixed(n: u8) -> Id {
    Id::from_bytes([n; 16])
}

fn fill(layout: &bilbo_store::layout::Layout) -> (Store, Id, Id) {
    let mut store = open(layout);
    let a = id_of(&keep_as(&mut store, "a.md", b"one\n", Some(fixed(1))));
    let b = id_of(&keep_as(&mut store, "b.md", b"two\n", Some(fixed(2))));
    (store, a, b)
}

fn tags_only(store: &mut Store, id: Id) {
    let mut meta: Meta = store.meta(id).expect("meta").expect("some");
    meta.labels.tags.set(tag("a"), Some(PERSON));
    store.set_meta(id, &meta).expect("set_meta");
}

#[test]
fn set_meta_that_changes_only_tags_moves_no_cursor() {
    let scratch = Scratch::new("tag_cursor");
    let layout = scratch.layout();
    let (mut store, a, _) = fill(&layout);
    let (_, cursor) = store.changes(&Cursor::start()).expect("changes");
    tags_only(&mut store, a);
    let (changes, after) = store.changes(&cursor).expect("changes");
    assert_eq!(changes, Vec::<Change>::new());
    assert_eq!(after, cursor);
    drop(store);
    let store = open(&layout);
    let (changes, _) = store.changes(&cursor).expect("changes");
    assert_eq!(changes, Vec::<Change>::new());
    drop(store);

    let other = Scratch::new("tag_cursor_plain");
    let plain = other.layout();
    let (store, _, _) = fill(&plain);
    drop(store);
    for layout in [&layout, &plain] {
        fs::remove_file(catalog(layout)).expect("delete the catalog");
    }
    let tagged = open(&layout);
    let untagged = open(&plain);
    assert_eq!(
        tagged.changes(&Cursor::start()).expect("changes").0,
        untagged.changes(&Cursor::start()).expect("changes").0
    );
}

#[test]
fn tagged_names_the_carriers_in_id_order_and_tags_counts_them_by_kind() {
    let scratch = Scratch::new("tag_listing");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let one = id_of(&keep_tagged(
        &mut store,
        "a.md",
        "markdown",
        b"one\n",
        &tags_of(&[("a", PERSON), ("b", MODEL)]),
    ));
    let two = id_of(&keep_tagged(
        &mut store,
        "b.md",
        "markdown",
        b"two\n",
        &tags_of(&[("a", MODEL)]),
    ));
    let three = id_of(&keep_tagged(
        &mut store,
        "c.md",
        "session",
        b"three\n",
        &tags_of(&[("a", PERSON)]),
    ));
    let mut carriers = vec![one, two, three];
    carriers.sort();
    assert_eq!(store.tagged(&tag("a")).expect("tagged"), carriers);
    assert_eq!(store.tagged(&tag("b")).expect("tagged"), vec![one]);
    assert_eq!(
        store.tags().expect("tags"),
        vec![
            Tagged {
                tag: tag("a"),
                kind: "markdown".into(),
                entries: 2
            },
            Tagged {
                tag: tag("a"),
                kind: "session".into(),
                entries: 1
            },
            Tagged {
                tag: tag("b"),
                kind: "markdown".into(),
                entries: 1
            },
        ]
    );
}

#[test]
fn a_removed_name_comes_back_from_the_meta_and_is_in_no_count() {
    let scratch = Scratch::new("tag_removed");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let given = tags_of(&[("a", Standing::Removed)]);
    let id = id_of(&keep_tagged(
        &mut store, "a.md", "markdown", b"one\n", &given,
    ));
    assert_eq!(tags_of_entry(&store, id), given);
    assert_eq!(store.tagged(&tag("a")).expect("tagged"), vec![]);
    assert_eq!(store.tags().expect("tags"), vec![]);
}

#[test]
fn a_read_only_open_reads_the_tags_and_writes_nothing() {
    let scratch = Scratch::new("tag_read_only");
    let layout = scratch.layout();
    let given = tags_of(&[("a", PERSON)]);
    let mut store = open(&layout);
    let id = id_of(&keep_tagged(
        &mut store, "a.md", "markdown", b"one\n", &given,
    ));
    drop(store);
    let before = snapshot_of(&layout);
    let read_only = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only");
    assert_eq!(tags_of_entry(&read_only, id), given);
    assert_eq!(read_only.tagged(&tag("a")).expect("tagged"), vec![id]);
    assert_eq!(
        read_only
            .read(id, Generation(1), RecordRange { first: 1, count: 1 })
            .expect("read")
            .concat(),
        b"one\n"
    );
    drop(read_only);
    assert_eq!(snapshot_of(&layout), before);

    let other = Scratch::new("tag_read_only_format_1");
    let old = other.layout();
    let mut store = open(&old);
    keep(&mut store, "a.md", b"one\n");
    drop(store);
    let mut bytes = fs::read(open_seg(&old)).expect("open.seg");
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    fs::write(open_seg(&old), &bytes).expect("write");
    let before = snapshot_of(&old);
    let read_only = Store::open_read_only(&old, sealing(), &mut |_| {}).expect("read-only");
    drop(read_only);
    assert_eq!(snapshot_of(&old), before);
    assert_eq!(fs::read(open_seg(&old)).expect("open.seg"), bytes);
}
