use std::fs;

use bilbo_store::entries::{Entries, Generation, Meta};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{Labels, Relabel, StoredLabels};
use bilbo_store::segments::Store;

use crate::scratch::{
    Scratch, capture_label, catalog, filed, id_of, keep, keep_as, open, sealing, words,
};

#[test]
fn an_extent_is_a_generation_and_how_many_records_it_holds() {
    let scratch = Scratch::new("lookup_extent");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "a.md", b"one\ntwo\n"));
    assert_eq!(
        store.extent(id, None).expect("extent"),
        Some((Generation(1), 2))
    );
    keep(&mut store, "a.md", b"one\ntwo\nthree\n");
    assert_eq!(
        store.extent(id, None).expect("extent"),
        Some((Generation(1), 3))
    );
    keep(&mut store, "a.md", b"other\n");
    assert_eq!(
        store.extent(id, None).expect("extent"),
        Some((Generation(2), 1))
    );
    assert_eq!(
        store.extent(id, Some(Generation(1))).expect("extent"),
        Some((Generation(1), 3))
    );
    assert_eq!(
        store.extent(id, Some(Generation(2))).expect("extent"),
        Some((Generation(2), 1))
    );
    assert_eq!(store.extent(id, Some(Generation(3))).expect("extent"), None);
    assert_eq!(store.extent(id, Some(Generation(0))).expect("extent"), None);
    let unknown = Id::from_bytes([9; 16]);
    assert_eq!(store.extent(unknown, None).expect("extent"), None);
    assert_eq!(
        store.extent(unknown, Some(Generation(1))).expect("extent"),
        None
    );
}

#[test]
fn an_entry_kept_from_an_empty_file_has_a_generation_with_no_record() {
    let scratch = Scratch::new("lookup_empty");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let id = id_of(&keep(&mut store, "empty.md", b""));
    assert_eq!(
        store.extent(id, None).expect("extent"),
        Some((Generation(1), 0))
    );
    assert_eq!(
        store.extent(id, Some(Generation(1))).expect("extent"),
        Some((Generation(1), 0))
    );
}

#[test]
fn entries_are_found_by_a_part_of_their_capture_label_in_any_case_and_in_id_order() {
    let scratch = Scratch::new("lookup_captures");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let first = id_of(&keep(&mut store, "5C3A9E1F-session.md", b"x\n"));
    let second = id_of(&keep(&mut store, "rollout-5c3a9e1f.md", b"y\n"));
    keep(&mut store, "other_100%.md", b"z\n");
    let found = |part: &str| store.captures_with(part).expect("captures");
    let mut both = vec![
        (first, capture_label("5C3A9E1F-session.md")),
        (second, capture_label("rollout-5c3a9e1f.md")),
    ];
    both.sort_by_key(|(id, _)| *id);
    assert_eq!(found("5c3a9e1f"), both);
    assert_eq!(found("5C3A9E1F"), both);
    assert_eq!(
        found("5c3a9e1f-"),
        [(first, capture_label("5C3A9E1F-session.md"))]
    );
    assert_eq!(found("5c3a9e1e"), []);
    assert_eq!(found("%").len(), 1, "a percent sign is no wildcard");
    assert_eq!(found("_").len(), 1, "an underscore is no wildcard");
    assert_eq!(found("/notes/").len(), 3);
}

#[test]
fn the_lookups_answer_the_same_from_a_rebuilt_catalog_and_from_a_read_only_store() {
    let scratch = Scratch::new("lookup_rebuilt");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let sealed = id_of(&keep(&mut store, "5c3a9e1f-a.md", b"one\ntwo\n"));
    store.close().expect("close");
    let mut store = open(&layout);
    keep(&mut store, "5c3a9e1f-a.md", b"changed\n");
    let open_only = id_of(&keep(&mut store, "5c3a9e1f-b.md", b"three\n"));
    drop(store);
    let mut expected = vec![
        (sealed, capture_label("5c3a9e1f-a.md")),
        (open_only, capture_label("5c3a9e1f-b.md")),
    ];
    expected.sort_by_key(|(id, _)| *id);
    let check = |store: &Store, case: &str| {
        assert_eq!(
            store.captures_with("5c3a9e1f").expect("captures"),
            expected,
            "{case}"
        );
        assert_eq!(
            store.extent(sealed, None).expect("extent"),
            Some((Generation(2), 1)),
            "{case}"
        );
        assert_eq!(
            store.extent(sealed, Some(Generation(1))).expect("extent"),
            Some((Generation(1), 2)),
            "{case}"
        );
    };
    check(
        &Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only"),
        "read-only, the catalog current",
    );
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    check(
        &Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only"),
        "read-only, no catalog",
    );
    assert!(!catalog(&layout).exists());
    check(&open(&layout), "a writer that rebuilt the catalog");
}

fn id(text: &str) -> Id {
    Id::parse(text).expect("id")
}

fn with_bits(id: Id, flip: u32) -> Id {
    Id::from_bytes((u128::from_be_bytes(id.to_bytes()) ^ (1 << flip)).to_be_bytes())
}

#[test]
fn entries_are_found_by_the_ends_of_their_ids() {
    let scratch = Scratch::new("lookup_shorts");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let ids = [
        id("01JC5Q3V7XKM4P8R2T6W9YB0DE"),
        id("01JC5Q3V7XKM4P8R2T6W9YB0DF"),
        id("01JC5Q3V7XKM4P8R2T6W9YB0DG"),
    ];
    for (index, id) in ids.iter().enumerate() {
        keep_as(&mut store, &format!("{index}.md"), b"x\n", Some(*id));
    }
    let found = |shorts: &[Short]| store.by_shorts(shorts).expect("by_shorts");
    assert_eq!(found(&[ids[1].short()]), [ids[1]]);
    assert_eq!(found(&[ids[2].short(), ids[0].short()]), [ids[0], ids[2]]);
    let absent = Short::parse("2T6W9YB0DZ").expect("short");
    assert_eq!(found(&[ids[0].short(), absent]), [ids[0]]);
    assert_eq!(found(&[]), []);
}

#[test]
fn ids_that_share_six_bytes_and_differ_above_them_are_told_apart() {
    let scratch = Scratch::new("lookup_six_bytes");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let base = id("01JC5Q3V7XKM4P8R2T6W9YB0DE");
    let above = with_bits(base, 48);
    let same = with_bits(base, 50);
    for (name, id) in [("a.md", base), ("b.md", above), ("c.md", same)] {
        keep_as(&mut store, name, b"x\n", Some(id));
    }
    assert_ne!(base.short(), above.short());
    assert_eq!(base.short(), same.short());
    assert_eq!(store.by_shorts(&[above.short()]).expect("above"), [above]);
    let mut both = vec![base, same];
    both.sort();
    assert_eq!(store.by_shorts(&[base.short()]).expect("base"), both);
}

#[test]
fn a_thousand_short_ids_are_one_lookup() {
    let scratch = Scratch::new("lookup_thousand");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let kept = [
        id("01JC5Q3V7XKM4P8R2T6W9YB0DE"),
        id("01JC5Q3V7XKM4P8R2T6W9YB0DF"),
        id("01JC5Q3V7XKM4P8R2T6W9YB0DG"),
    ];
    for (index, id) in kept.iter().enumerate() {
        keep_as(&mut store, &format!("{index}.md"), b"x\n", Some(*id));
    }
    let mut shorts: Vec<Short> = (1u128..=997)
        .map(|n| Id::from_bytes(n.to_be_bytes()).short())
        .collect();
    shorts.extend(kept.iter().map(|id| id.short()));
    assert_eq!(shorts.len(), 1000);
    assert_eq!(store.by_shorts(&shorts).expect("by_shorts"), kept);
}

#[test]
fn a_reader_finds_an_entry_by_its_short_id_with_and_without_a_kept_catalog() {
    let scratch = Scratch::new("lookup_shorts_read_only");
    let layout = scratch.layout();
    let held = id("01JC5Q3V7XKM4P8R2T6W9YB0DE");
    let mut store = open(&layout);
    keep_as(&mut store, "a.md", b"one\n", Some(held));
    drop(store);
    let check = |case: &str| {
        let store = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only");
        assert_eq!(
            store.by_shorts(&[held.short()]).expect("by_shorts"),
            [held],
            "{case}"
        );
    };
    check("the catalog current");
    fs::remove_file(catalog(&layout)).expect("delete the catalog");
    check("no catalog");
    assert!(!catalog(&layout).exists());
}

#[test]
fn an_entry_is_found_by_its_capture_label() {
    let scratch = Scratch::new("lookup_by_capture");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    keep(&mut store, "b.md", b"two\n");
    let head = store.by_capture(&capture_label("a.md")).expect("head");
    assert_eq!(head.map(|head| head.id), Some(a));
    assert!(
        store
            .by_capture(&capture_label("c.md"))
            .expect("head")
            .is_none()
    );
}

#[test]
fn text_labels_survive_a_reopen_and_a_reader_reads_them() {
    let scratch = Scratch::new("lookup_text_labels");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    let b = id_of(&keep(&mut store, "b.md", b"two\n"));
    store
        .relabel(&[filed(a, &["home me src shed"]), filed(b, &["home me docs"])])
        .expect("relabel");
    store.close().expect("close");
    let store = open(&layout);
    assert_eq!(store.with_words(&words("src shed")).expect("find"), [a]);
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(store.with_words(&words("home me")).expect("find"), both);
    drop(store);
    let reader = Store::open_read_only(&layout, sealing(), &mut |_| {}).expect("read-only open");
    assert_eq!(reader.with_words(&words("docs")).expect("find"), [b]);
}

#[test]
fn captures_with_looks_at_capture_labels_only() {
    let scratch = Scratch::new("lookup_captures_only");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let a = id_of(&keep(&mut store, "a.md", b"one\n"));
    store.relabel(&[filed(a, &["zebra"])]).expect("relabel");
    assert_eq!(store.captures_with("zebra").expect("found"), []);
    assert_eq!(store.captures_with("a.md").expect("found").len(), 1);
}

#[test]
fn a_subagent_of_a_subagent_belongs_to_the_session() {
    let scratch = Scratch::new("lookup_deep");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let session = id_of(&keep(&mut store, "session.md", b"s\n"));
    let first = id_of(&keep(&mut store, "first.md", b"f\n"));
    let second = id_of(&keep(&mut store, "second.md", b"g\n"));
    keep(&mut store, "other.md", b"o\n");
    for (entry, above) in [(first, session), (second, first)] {
        let meta = store.meta(entry).expect("meta").expect("some");
        let meta = Meta {
            labels: StoredLabels {
                id: Some(above),
                ..meta.labels.clone()
            },
            ..meta
        };
        store.set_meta(entry, &meta).expect("set_meta");
    }
    let mut below = vec![first, second];
    below.sort();
    assert_eq!(store.belonging_to(&[session]).expect("found"), below);
    assert_eq!(store.belonging_to(&[first]).expect("found"), [second]);
}
