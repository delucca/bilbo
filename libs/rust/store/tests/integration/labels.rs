use bilbo_store::entries::{Capture, Entries, StoreError};
use bilbo_store::id::Id;
use bilbo_store::labels::{Filed, Labels, Relabel, Worded};

use crate::scratch::{
    Scratch, capture_label, catalog, count, ends, filed, label_rows, label_tables, open, sql, words,
};

fn at(n: u8) -> Id {
    Id::from_bytes([n; 16])
}

#[test]
fn relabelling_an_entry_replaces_its_text_labels_whole() {
    let scratch = Scratch::new("labels_whole");
    let layout = scratch.layout();
    let store = open(&layout);
    store
        .relabel(&[filed(at(1), &["a", "b"])])
        .expect("relabel");
    store
        .relabel(&[filed(at(1), &["b", "c"])])
        .expect("relabel");
    assert_eq!(store.with_words(&words("a")).expect("find"), vec![]);
    assert_eq!(store.with_words(&words("b")).expect("find"), vec![at(1)]);
    assert_eq!(store.with_words(&words("c")).expect("find"), vec![at(1)]);
    assert_eq!(label_rows(&layout).0, 2);
    assert_eq!(
        count(&layout, "SELECT count(*) FROM label_words WHERE word = 'a'"),
        0
    );
}

#[test]
fn relabelling_some_entries_leaves_the_others() {
    let scratch = Scratch::new("labels_some");
    let store = open(&scratch.layout());
    store
        .relabel(&[filed(at(1), &["a"]), filed(at(2), &["b"])])
        .expect("relabel");
    store.relabel(&[filed(at(1), &["c"])]).expect("relabel");
    assert_eq!(store.with_words(&words("a")).expect("find"), vec![]);
    assert_eq!(store.with_words(&words("b")).expect("find"), vec![at(2)]);
    assert_eq!(store.with_words(&words("c")).expect("find"), vec![at(1)]);
}

#[test]
fn an_entry_given_no_label_loses_its_rows() {
    let scratch = Scratch::new("labels_none");
    let layout = scratch.layout();
    let store = open(&layout);
    store
        .relabel(&[filed(at(1), &["a b"]), filed(at(2), &["c"])])
        .expect("relabel");
    store.relabel(&[filed(at(1), &[])]).expect("relabel");
    assert_eq!(store.with_words(&words("a b")).expect("find"), vec![]);
    assert_eq!(store.with_words(&words("c")).expect("find"), vec![at(2)]);
    assert_eq!(label_rows(&layout), (1, 1, 1));
}

#[test]
fn relabelling_writes_a_labels_words_again() {
    let scratch = Scratch::new("labels_again");
    let store = open(&scratch.layout());
    store.relabel(&[filed(at(1), &["a b"])]).expect("relabel");
    store
        .relabel(&[Filed {
            entry: at(2),
            labels: vec![Worded {
                text: "a b".into(),
                words: words("x"),
            }],
        }])
        .expect("relabel");
    assert_eq!(
        store.with_words(&words("x")).expect("find"),
        vec![at(1), at(2)]
    );
    assert_eq!(store.with_words(&words("a")).expect("find"), vec![]);
}

#[test]
fn a_text_two_entries_share_is_kept_once_and_names_both() {
    let scratch = Scratch::new("labels_shared");
    let layout = scratch.layout();
    let store = open(&layout);
    store
        .relabel(&[filed(at(2), &["shed"]), filed(at(1), &["shed"])])
        .expect("relabel");
    assert_eq!(label_rows(&layout).0, 1);
    assert_eq!(
        store.with_words(&words("shed")).expect("find"),
        vec![at(1), at(2)]
    );
    store.relabel(&[filed(at(1), &[])]).expect("relabel");
    assert_eq!(store.with_words(&words("shed")).expect("find"), vec![at(2)]);
    assert_eq!(label_rows(&layout).0, 1);
}

#[test]
fn words_next_to_each_other_and_in_order_inside_one_label_name_its_entries() {
    let scratch = Scratch::new("labels_adjacent");
    let store = open(&scratch.layout());
    store
        .relabel(&[
            filed(at(1), &["home me src shed"]),
            filed(at(2), &["a b a c"]),
        ])
        .expect("relabel");
    for (found, entry) in [
        ("src", at(1)),
        ("me src", at(1)),
        ("home me src shed", at(1)),
        ("a c", at(2)),
        ("a b a", at(2)),
    ] {
        assert_eq!(
            store.with_words(&words(found)).expect("find"),
            vec![entry],
            "{found}"
        );
    }
}

#[test]
fn words_apart_out_of_order_or_across_two_labels_name_nothing() {
    let scratch = Scratch::new("labels_apart");
    let store = open(&scratch.layout());
    store
        .relabel(&[filed(at(1), &["a b c", "d", "e"])])
        .expect("relabel");
    for apart in ["a c", "b a", "c b a", "d e", "e d", "c d", "a b c d"] {
        assert_eq!(
            store.with_words(&words(apart)).expect("find"),
            vec![],
            "{apart}"
        );
    }
}

#[test]
fn no_word_names_no_entry() {
    let scratch = Scratch::new("labels_no_word");
    let store = open(&scratch.layout());
    store.relabel(&[filed(at(1), &["a"])]).expect("relabel");
    assert_eq!(store.with_words(&[]).expect("find"), vec![]);
}

#[test]
fn the_entries_that_belong_to_some_are_read_from_the_id_labels() {
    let scratch = Scratch::new("labels_belonging");
    let mut store = open(&scratch.layout());
    for (n, above) in [
        (1, None),
        (2, Some(1)),
        (3, Some(2)),
        (4, None),
        (5, Some(4)),
    ] {
        let label = capture_label(&format!("e{n}.md"));
        store
            .capture(&Capture {
                label: &label,
                kind: "markdown",
                id: Some(at(n)),
                id_label: above.map(at),
                stamp: None,
                bytes: b"x\n",
                ends: &ends(b"x\n"),
            })
            .expect("capture");
    }
    assert_eq!(
        store.belonging_to(&[at(1)]).expect("found"),
        vec![at(2), at(3)]
    );
    assert_eq!(store.belonging_to(&[at(2)]).expect("found"), vec![at(3)]);
    assert_eq!(
        store.belonging_to(&[at(1), at(4)]).expect("found"),
        vec![at(2), at(3), at(5)]
    );
    assert_eq!(store.belonging_to(&[at(3)]).expect("found"), vec![]);
    assert_eq!(store.belonging_to(&[]).expect("found"), vec![]);
}

#[test]
fn a_labels_words_are_reached_through_their_index() {
    let scratch = Scratch::new("labels_index");
    let layout = scratch.layout();
    let store = open(&layout);
    store.relabel(&[filed(at(1), &["a b"])]).expect("relabel");
    let conn = rusqlite::Connection::open(catalog(&layout)).expect("open the catalog");
    for statement in [
        "SELECT word, at FROM label_words WHERE label = 1",
        "DELETE FROM label_words WHERE label = 1",
    ] {
        let mut plan = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {statement}"))
            .expect("prepare");
        let details: Vec<String> = plan
            .query_map([], |row| row.get(3))
            .expect("plan")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("label_words_by_label")),
            "{statement}: {details:?}"
        );
    }
}

#[test]
fn a_call_that_names_an_entry_twice_leaves_what_the_last_item_gave() {
    let scratch = Scratch::new("labels_twice");
    let layout = scratch.layout();
    let store = open(&layout);
    let (a, b) = (at(1), at(2));
    let orphans = "SELECT count(*) FROM label_words WHERE label NOT IN (SELECT id FROM labels)";

    store
        .relabel(&[filed(a, &["x"]), filed(a, &[])])
        .expect("relabel");
    assert_eq!(store.with_words(&words("x")).expect("find"), vec![]);
    assert_eq!(label_rows(&layout), (0, 0, 0));
    assert_eq!(count(&layout, orphans), 0);

    store
        .relabel(&[filed(a, &["y"]), filed(a, &["z"])])
        .expect("relabel");
    assert_eq!(store.with_words(&words("y")).expect("find"), vec![]);
    assert_eq!(store.with_words(&words("z")).expect("find"), vec![a]);
    assert_eq!(label_rows(&layout), (1, 1, 1));
    assert_eq!(count(&layout, orphans), 0);

    store.relabel(&[filed(b, &["w"])]).expect("relabel");
    assert_eq!(store.with_words(&words("w")).expect("find"), vec![b]);
    assert_eq!(store.with_words(&words("x")).expect("find"), vec![]);
    assert_eq!(store.with_words(&words("y")).expect("find"), vec![]);
    assert_eq!(count(&layout, orphans), 0);
}

#[test]
fn a_relabel_that_fails_partway_changes_no_row() {
    let scratch = Scratch::new("labels_partway");
    let layout = scratch.layout();
    let store = open(&layout);
    let (a, b) = (at(1), at(2));
    store
        .relabel(&[filed(a, &["old shed"]), filed(b, &["kept"])])
        .expect("relabel");
    sql(
        &layout,
        "CREATE TRIGGER refuse BEFORE INSERT ON label_words WHEN NEW.word = 'boom' \
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    );
    let before = label_tables(&layout);
    assert!(matches!(
        store.relabel(&[filed(a, &["new"]), filed(b, &["boom"])]),
        Err(StoreError::Catalog { .. })
    ));
    assert_eq!(label_tables(&layout), before);
    assert_eq!(store.with_words(&words("old")).expect("find"), vec![a]);
    assert_eq!(store.with_words(&words("kept")).expect("find"), vec![b]);
    assert_eq!(store.with_words(&words("new")).expect("find"), vec![]);
    assert_eq!(store.with_words(&words("boom")).expect("find"), vec![]);
}
