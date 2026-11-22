use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use bilbo_entries::tag::{Applied, Change, Error, declare, unused};
use bilbo_store::entries::Meta;
use bilbo_store::labels::{By, CaptureLabel, Standing, StoredLabels, Tag, Tags};

use crate::fakes::{ADOPTED, FakeEntries, id};

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
