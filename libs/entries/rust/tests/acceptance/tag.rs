use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use bilbo_entries::read;
use bilbo_entries::read::refs::Target;
use bilbo_entries::tag::{
    Applied, Carrier, Change, ChangeError, Error, Request, carriers, change, declare, unused,
};
use bilbo_store::entries::Meta;
use bilbo_store::id::Id;
use bilbo_store::labels::{By, CaptureLabel, Standing, StoredLabels, Tag, Tags};

use crate::fakes::{ADOPTED, FakeEntries, id};

fn twin(n: u8) -> Id {
    let mut bytes = [n; 16];
    bytes[0] = 0x02;
    Id::from_bytes(bytes)
}

fn asking(targets: Vec<Target>, add: &[&str], remove: &[&str]) -> Request {
    Request {
        targets,
        add: names(add),
        remove: names(remove),
        listed: 5,
    }
}

fn entry(n: u8) -> Target {
    Target::Entry(id(n))
}

fn tag(name: &str) -> Tag {
    Tag::new(name).expect("a tag")
}

fn names(names: &[&str]) -> BTreeSet<Tag> {
    names.iter().map(|name| tag(name)).collect()
}

fn carried(pairs: &[(&str, Standing)]) -> Tags {
    let mut tags = Tags::default();
    for (name, standing) in pairs {
        tags.set(tag(name), Some(*standing));
    }
    tags
}

const PERSON: Standing = Standing::Carried(By::Person);

fn adding(add: &BTreeSet<Tag>) -> Change<'_> {
    static NONE: BTreeSet<Tag> = BTreeSet::new();
    Change::Person { add, remove: &NONE }
}

#[test]
fn declare_writes_one_frame_when_the_standings_change_and_none_when_they_do_not() {
    let mut entries = FakeEntries::new().held(id(1), "fake", Tags::default());
    let add = names(&["a"]);

    let first = declare(id(1), &adding(&add), 32, &mut entries).expect("first");
    assert!(first.wrote);
    assert_eq!(first.bytes, ADOPTED);
    assert_eq!(entries.adopted.len(), 1);

    let again = declare(id(1), &adding(&add), 32, &mut entries).expect("again");
    assert!(!again.wrote);
    assert_eq!(again.bytes, 0);
    assert_eq!(entries.adopted.len(), 1);
}

#[test]
fn declare_gives_back_what_was_left_off_when_nothing_is_written() {
    let before = carried(&[("a", PERSON)]);
    let mut entries = FakeEntries::new().held(id(1), "fake", before.clone());
    let picks = [tag("b")];
    let change = Change::Model {
        picks: &picks,
        takes_back: false,
    };

    let declared = declare(id(1), &change, 1, &mut entries).expect("declared");

    assert!(!declared.wrote);
    assert_eq!(
        declared.applied,
        Applied {
            tags: before.clone(),
            added: Vec::new(),
            taken: Vec::new(),
            left_off: 1,
        }
    );
    assert_eq!(entries.metas[&id(1)].labels.tags, before);
    assert!(entries.adopted.is_empty());
}

#[test]
fn declare_of_an_entry_the_store_does_not_hold_is_gone() {
    let mut entries = FakeEntries::new();
    let add = names(&["a"]);

    let error = declare(id(9), &adding(&add), 32, &mut entries).expect_err("gone");

    assert!(matches!(error, Error::Gone(gone) if gone == id(9)));
    assert!(entries.adopted.is_empty());
}

#[test]
fn declare_keeps_the_capture_label_the_id_label_the_kind_and_the_time() {
    let before = Meta {
        id: id(1),
        kind: "session".to_owned(),
        labels: StoredLabels {
            capture: CaptureLabel::new("file:/t/a.fake".into()),
            id: Some(id(2)),
            tags: Tags::default(),
        },
        captured: SystemTime::UNIX_EPOCH + Duration::from_secs(5),
    };
    let mut entries = FakeEntries::new();
    entries.metas.insert(id(1), before.clone());
    let add = names(&["a"]);

    declare(id(1), &adding(&add), 32, &mut entries).expect("declared");

    assert_eq!(
        entries.metas[&id(1)],
        Meta {
            labels: StoredLabels {
                tags: carried(&[("a", PERSON)]),
                ..before.labels.clone()
            },
            ..before
        }
    );
}

#[test]
fn unused_names_the_tags_no_entry_carries_in_name_order() {
    let entries = FakeEntries::new()
        .held(id(1), "fake", carried(&[("b", PERSON)]))
        .held(id(2), "fake", carried(&[("c", Standing::Removed)]));

    let found = unused(&names(&["b", "a", "c"]), &entries).expect("unused");

    assert_eq!(found, vec![tag("a"), tag("c")]);
}

#[test]
fn change_checks_every_entry_before_the_first_write() {
    let mut entries = FakeEntries::new()
        .held(id(1), "fake", Tags::default())
        .held(id(2), "fake", Tags::default())
        .held(id(3), "fake", carried(&[("x", PERSON)]));

    let error = change(
        &asking(vec![entry(1), entry(2), entry(3)], &["a"], &[]),
        1,
        &mut entries,
    )
    .expect_err("the third is at the cap");

    assert!(matches!(error, ChangeError::Tag(Error::Full(full)) if full == id(3)));
    assert!(entries.adopted.is_empty());
}

#[test]
fn change_of_a_target_that_names_no_entry_or_several_writes_nothing() {
    let held = || {
        FakeEntries::new()
            .resolving()
            .held(id(1), "fake", Tags::default())
            .held(id(2), "fake", Tags::default())
            .held(id(3), "fake", Tags::default())
            .held(twin(3), "fake", Tags::default())
    };
    let mut entries = held();
    let error = change(
        &asking(vec![entry(1), entry(2), entry(9)], &["a"], &[]),
        32,
        &mut entries,
    )
    .expect_err("no such entry");
    assert!(
        matches!(error, ChangeError::Ref(read::Error::NoEntry(Target::Entry(gone))) if gone == id(9))
    );
    assert!(entries.adopted.is_empty());

    let mut entries = held();
    let error = change(
        &asking(
            vec![entry(1), entry(2), Target::Short(id(9).short())],
            &["a"],
            &[],
        ),
        32,
        &mut entries,
    )
    .expect_err("no id ends so");
    assert!(
        matches!(error, ChangeError::Ref(read::Error::NoEntry(Target::Short(short))) if short == id(9).short())
    );
    assert!(entries.adopted.is_empty());

    let mut entries = held();
    let error = change(
        &asking(
            vec![entry(1), entry(2), Target::Short(id(3).short())],
            &["a"],
            &[],
        ),
        32,
        &mut entries,
    )
    .expect_err("two ids end so");
    assert!(matches!(
        error,
        ChangeError::Ref(read::Error::Ambiguous { .. })
    ));
    assert!(entries.adopted.is_empty());
}

#[test]
fn change_counts_an_entry_changed_when_a_frame_was_written_for_it() {
    let mut entries = FakeEntries::new()
        .held(id(1), "fake", Tags::default())
        .held(
            id(2),
            "fake",
            carried(&[("a", Standing::Carried(By::Model))]),
        )
        .held(id(3), "fake", carried(&[("a", PERSON)]));

    let output = change(
        &asking(vec![entry(1), entry(2), entry(3)], &["a"], &[]),
        32,
        &mut entries,
    )
    .expect("changed");

    assert_eq!((output.changed, output.unchanged), (2, 1));
    assert_eq!(entries.adopted.len(), 2);
}

#[test]
fn change_names_the_tags_no_entry_carried_before() {
    let mut entries = FakeEntries::new()
        .held(id(1), "fake", carried(&[("b", PERSON)]))
        .held(id(2), "fake", Tags::default());

    let output = change(
        &asking(vec![entry(2)], &["c", "b", "a"], &[]),
        32,
        &mut entries,
    )
    .expect("changed");

    assert_eq!(output.new, vec![tag("a"), tag("c")]);
}

#[test]
fn a_remove_of_a_tag_the_entry_does_not_carry_changes_nothing() {
    let mut entries = FakeEntries::new().held(id(1), "fake", Tags::default());

    let output =
        change(&asking(vec![entry(1)], &[], &["a"]), 32, &mut entries).expect("nothing changes");

    assert_eq!((output.changed, output.unchanged), (0, 1));
    assert!(entries.adopted.is_empty());
    assert_eq!(entries.metas[&id(1)].labels.tags, Tags::default());
}

#[test]
fn carriers_are_in_id_order_with_kind_capture_label_and_whose() {
    let mut entries = FakeEntries::new()
        .held(
            id(2),
            "session",
            carried(&[("a", Standing::Carried(By::Model))]),
        )
        .held(id(1), "markdown", carried(&[("a", PERSON)]))
        .held(id(3), "markdown", carried(&[("b", PERSON)]));
    let label = CaptureLabel::new("file:/n/a.md".into()).expect("a label");
    entries
        .metas
        .get_mut(&id(1))
        .expect("a meta")
        .labels
        .capture = Some(label.clone());

    let found = carriers(&tag("a"), &entries, &entries).expect("carriers");

    assert_eq!(
        found,
        vec![
            Carrier {
                id: id(1),
                kind: "markdown".to_owned(),
                capture: Some(label),
                by: By::Person,
            },
            Carrier {
                id: id(2),
                kind: "session".to_owned(),
                capture: None,
                by: By::Model,
            },
        ]
    );
}

#[test]
fn an_entry_named_twice_is_changed_once() {
    let mut entries = FakeEntries::new()
        .resolving()
        .held(id(1), "fake", Tags::default());

    let output = change(
        &asking(vec![entry(1), Target::Short(id(1).short())], &["a"], &[]),
        32,
        &mut entries,
    )
    .expect("changed");

    assert_eq!((output.changed, output.unchanged), (1, 0));
    assert_eq!(entries.adopted.len(), 1);
}
