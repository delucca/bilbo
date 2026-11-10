use std::fs;

use bilbo_store::entries::{Entries, Generation};
use bilbo_store::id::Id;
use bilbo_store::segments::Store;

use crate::scratch::{Scratch, catalog, id_of, keep, open, origin, sealing};

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
fn entries_are_found_by_a_part_of_their_origin_in_any_case_and_in_id_order() {
    let scratch = Scratch::new("lookup_origins");
    let layout = scratch.layout();
    let mut store = open(&layout);
    let first = id_of(&keep(&mut store, "5C3A9E1F-session.md", b"x\n"));
    let second = id_of(&keep(&mut store, "rollout-5c3a9e1f.md", b"y\n"));
    keep(&mut store, "other_100%.md", b"z\n");
    let found = |part: &str| store.origins_with(part).expect("origins");
    let mut both = vec![
        (first, origin("5C3A9E1F-session.md")),
        (second, origin("rollout-5c3a9e1f.md")),
    ];
    both.sort_by_key(|(id, _)| *id);
    assert_eq!(found("5c3a9e1f"), both);
    assert_eq!(found("5C3A9E1F"), both);
    assert_eq!(found("5c3a9e1f-"), [(first, origin("5C3A9E1F-session.md"))]);
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
        (sealed, origin("5c3a9e1f-a.md")),
        (open_only, origin("5c3a9e1f-b.md")),
    ];
    expected.sort_by_key(|(id, _)| *id);
    let check = |store: &Store, case: &str| {
        assert_eq!(
            store.origins_with("5c3a9e1f").expect("origins"),
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
