use std::time::{Duration, SystemTime};

use bilbo_entries::files::{Call, Files, Memory, Turning};
use bilbo_entries::forget::Request as Forget;
use bilbo_entries::forget::{Forgotten, forget};
use bilbo_entries::read::refs::Target;
use bilbo_entries::remember::{Error, Met, Outcome, Output};
use bilbo_store::entries::{Generation, Head, Mark, Stamp, StoreError};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Tags};

use crate::fakes::{ADOPTED, Counting, FakeEntries, FakeKind, Script, Write, id};
use crate::{by_person, carried, pass_of, request, run, tagged_request};

const WRITTEN: u64 = 100;

fn written() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(WRITTEN)
}

fn settled() -> SystemTime {
    written() + Duration::from_secs(10)
}

fn racy() -> SystemTime {
    written() + Duration::from_secs(1)
}

fn memory(files: &[(&str, &str)], clock: SystemTime) -> Memory {
    let mut memory = Memory::new().clock(clock);
    for (path, bytes) in files {
        memory = memory.file(*path, *bytes).modified(*path, written());
    }
    memory
}

fn label(path: &str) -> CaptureLabel {
    CaptureLabel::file(std::path::Path::new(path)).expect("label")
}

fn forgotten(n: u8, had: &str, held: &[u8]) -> Forgotten {
    Forgotten {
        id: fid(n),
        held: Mark::of(held),
        had: label(had),
    }
}

fn head(n: u8, generation: u32, records: u64) -> Head {
    Head {
        id: id(n),
        generation: Generation(generation),
        records,
        mark: Mark::of(b""),
        stamp: None,
    }
}

fn forgotten_entry(
    entries: FakeEntries,
    kind: &str,
    forgotten: &Forgotten,
    newest: (u32, u64),
) -> FakeEntries {
    let mut entries = entries.held(forgotten.id, kind, Tags::default());
    let made = forgotten.label();
    entries
        .metas
        .get_mut(&forgotten.id)
        .expect("meta")
        .labels
        .capture = Some(made.clone());
    let n = forgotten.id.to_bytes()[0];
    let mut newest_head = head(n, newest.0, newest.1);
    if newest.1 > 0 {
        newest_head.mark = forgotten.held;
    }
    entries.heads.insert(made.to_string(), newest_head);
    entries
}

fn gone(entries: FakeEntries, forgotten: &Forgotten) -> FakeEntries {
    forgotten_entry(entries, "fake", forgotten, (2, 0))
}

fn fid(n: u8) -> Id {
    id(100 + n)
}

fn with_id(n: u8, rest: &str) -> String {
    format!("id={}|{rest}", fid(n))
}

fn one_pass(files: &dyn Files, entries: &mut FakeEntries, path: &str) -> Output {
    pass_of(FakeKind::new(), files, entries, &[path]).remove(0)
}

fn one(output: &Output) -> &bilbo_entries::remember::Item {
    assert_eq!(output.items.len(), 1, "{:?}", output.items);
    &output.items[0]
}

fn own(n: u8, bytes: u64) -> Option<Met> {
    Some(Met::Own { id: fid(n), bytes })
}

fn metas(entries: &FakeEntries) -> Vec<&Write> {
    entries
        .writes
        .iter()
        .filter(|write| matches!(write, Write::Meta(..)))
        .collect()
}

fn bare_captures(entries: &FakeEntries) -> Vec<&str> {
    entries
        .seen
        .iter()
        .filter(|seen| seen.ends.is_empty())
        .map(|seen| seen.label.as_str())
        .collect()
}

#[test]
fn a_forgotten_file_at_its_path_is_passed_over_and_nothing_of_it_is_captured() {
    let files = memory(&[("/t/a.fake", "x|y|")], settled());
    let mut entries = gone(FakeEntries::new(), &forgotten(1, "/t/a.fake", b"x|y|"));

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    let item = one(&output);
    assert_eq!(item.outcome, Outcome::Unchanged(fid(1)));
    assert_eq!(item.forgotten, own(1, 0));
    assert_eq!(output.tally().new, 0);
    assert!(entries.seen.iter().all(|seen| seen.bytes.is_empty()));
    assert_eq!(output.stored_bytes, 0);
}

#[test]
fn a_forgotten_file_that_grew_is_still_passed_over() {
    let files = memory(&[("/t/a.fake", "x|y|z|w|")], settled());
    let mut entries = gone(FakeEntries::new(), &forgotten(1, "/t/a.fake", b"x|y|"));

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    let item = one(&output);
    assert_eq!(item.outcome, Outcome::Unchanged(fid(1)));
    assert_eq!(item.forgotten, own(1, 0));
    assert!(entries.seen.iter().all(|seen| seen.bytes.is_empty()));
}

#[test]
fn a_file_that_no_longer_begins_with_what_was_held_is_a_new_entry() {
    let files = memory(&[("/t/a.fake", "q|y|")], settled());
    let mut entries = gone(FakeEntries::new(), &forgotten(1, "/t/a.fake", b"x|y|"));
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert!(matches!(one(&output).outcome, Outcome::New(new) if new != fid(1)));
    assert_eq!(one(&output).forgotten, None);

    let files = memory(&[("/t/a.fake", "x|yy|")], settled());
    let mut entries = gone(FakeEntries::new(), &forgotten(1, "/t/a.fake", b"x|y"));
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert!(matches!(one(&output).outcome, Outcome::New(new) if new != fid(1)));
    assert_eq!(one(&output).forgotten, None);
}

#[test]
fn a_file_that_carries_a_forgotten_entrys_id_is_passed_over_wherever_it_is() {
    let body = with_id(1, "other bytes after it|");
    let files = memory(&[("/t/b.fake", body.as_str())], settled());
    let mut entries = gone(FakeEntries::new(), &forgotten(1, "/t/a.fake", b"x|y|"));

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    let item = one(&output);
    assert_eq!(item.outcome, Outcome::Unchanged(fid(1)));
    assert!(matches!(item.forgotten, Some(Met::Own { id: held, .. }) if held == fid(1)));
    assert_eq!(output.tally().refused, 0);
    assert_eq!(output.tally().new, 0);
}

#[test]
fn an_entry_that_held_nothing_guards_no_content() {
    let files = memory(&[("/t/a.fake", "x|")], settled());
    let mut entries = gone(FakeEntries::new(), &forgotten(1, "/t/a.fake", b""));
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert!(matches!(one(&output).outcome, Outcome::New(new) if new != fid(1)));

    let body = with_id(1, "x|");
    let files = memory(&[("/t/a.fake", body.as_str())], settled());
    let mut entries = gone(FakeEntries::new(), &forgotten(1, "/t/a.fake", b""));
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(one(&output).outcome, Outcome::Unchanged(fid(1)));
    assert_eq!(one(&output).forgotten, own(1, 0));
}

#[test]
fn a_forgotten_entry_of_another_kind_refuses_the_file_as_today() {
    let held = forgotten(1, "/t/a.fake", b"x|");
    let body = with_id(1, "x|");
    let files = memory(&[("/t/a.fake", body.as_str())], settled());
    let mut entries = forgotten_entry(FakeEntries::new(), "markdown", &held, (2, 0));

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    assert_eq!(
        one(&output).outcome,
        Outcome::Refused(format!("its id {} is held by {}", fid(1), held.label()))
    );
    assert_eq!(one(&output).forgotten, None);

    let mut entries = forgotten_entry(FakeEntries::new(), "markdown", &held, (2, 0));
    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t", &["a", "b"], 1),
    )
    .expect("run");
    assert_eq!(
        one(&output).outcome,
        Outcome::Refused(format!("its id {} is held by {}", fid(1), held.label()))
    );
}

#[test]
fn a_file_passed_over_keeps_its_stamp_and_is_not_read_the_next_time() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held);
    let first = Counting::new(memory(&[("/t/a.fake", "x|y|")], settled()));

    let output = run(FakeKind::new(), &first, &mut entries, &request("/t")).expect("first");

    assert_eq!(first.read_paths.borrow().len(), 1);
    assert_eq!(one(&output).forgotten, own(1, 0));
    assert_eq!(bare_captures(&entries), vec![held.label().as_str()]);
    let seen = entries.seen.last().expect("a capture");
    assert_eq!(
        seen.stamp,
        Some(Stamp {
            size: 4,
            modified: written()
        })
    );
    let after = entries.heads[held.label().as_str()];
    assert_eq!((after.generation, after.records), (Generation(2), 0));
    assert!(after.stamp.is_some());

    let second = Counting::new(memory(&[("/t/a.fake", "x|y|")], settled()));
    let output = run(FakeKind::new(), &second, &mut entries, &request("/t")).expect("second");
    assert_eq!(second.read_paths.borrow().len(), 0);
    assert_eq!(one(&output).outcome, Outcome::Unchanged(fid(1)));
    assert_eq!(one(&output).forgotten, None);
}

#[test]
fn a_file_written_within_two_seconds_is_read_again_the_next_time() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held);
    let first = Counting::new(memory(&[("/t/a.fake", "x|y|")], racy()));
    run(FakeKind::new(), &first, &mut entries, &request("/t")).expect("first");
    assert_eq!(entries.heads[held.label().as_str()].stamp, None);

    let second = Counting::new(memory(&[("/t/a.fake", "x|y|")], racy()));
    let output = run(FakeKind::new(), &second, &mut entries, &request("/t")).expect("second");
    assert_eq!(second.read_paths.borrow().len(), 1);
    assert_eq!(one(&output).forgotten, own(1, 0));
}

#[test]
fn a_half_forgotten_entry_is_passed_over_and_gets_no_capture() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = forgotten_entry(FakeEntries::new(), "fake", &held, (1, 2));
    let counting = Counting::new(memory(&[("/t/a.fake", "x|y|")], settled()));
    for _ in 0..2 {
        let output = run(FakeKind::new(), &counting, &mut entries, &request("/t")).expect("run");
        assert_eq!(one(&output).forgotten, own(1, 0));
    }
    assert_eq!(counting.read_paths.borrow().len(), 2);
    assert!(entries.seen.is_empty());

    let nothing = forgotten(2, "/t/b.fake", b"");
    let mut entries = forgotten_entry(FakeEntries::new(), "fake", &nothing, (0, 0));
    let body = with_id(2, "z|");
    let counting = Counting::new(memory(&[("/t/b.fake", body.as_str())], settled()));
    for _ in 0..2 {
        let output = run(FakeKind::new(), &counting, &mut entries, &request("/t")).expect("run");
        assert_eq!(one(&output).forgotten, own(2, 0));
        assert_eq!(one(&output).outcome, Outcome::Unchanged(fid(2)));
    }
    assert_eq!(counting.read_paths.borrow().len(), 2);
    assert!(entries.seen.is_empty());
}

#[test]
fn the_forgotten_entries_are_asked_for_once_a_keep_and_only_for_a_label_that_holds_none() {
    let files = memory(
        &[
            ("/t/n1.fake", "a|"),
            ("/t/n2.fake", "b|"),
            ("/t/n3.fake", "c|"),
            ("/t/k1.fake", "d|"),
            ("/t/k2.fake", "e|"),
        ],
        settled(),
    );
    let mut entries = FakeEntries::new()
        .kept_under(Some("file:/t/k1.fake"), id(5), "fake", Tags::default())
        .kept_under(Some("file:/t/k2.fake"), id(6), "fake", Tags::default());
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    let asked = entries.asked.borrow();
    assert_eq!(asked.iter().filter(|text| *text == "forgotten:").count(), 1);
    drop(asked);

    let files = memory(&[("/t/k1.fake", "d|"), ("/t/k2.fake", "e|")], settled());
    let mut entries = FakeEntries::new()
        .kept_under(Some("file:/t/k1.fake"), id(5), "fake", Tags::default())
        .kept_under(Some("file:/t/k2.fake"), id(6), "fake", Tags::default());
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert!(
        !entries
            .asked
            .borrow()
            .iter()
            .any(|text| text == "forgotten:")
    );

    let body = with_id(7, "a|");
    let files = memory(&[("/t/n.fake", body.as_str())], settled());
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(entries.meta_calls.get(), 1);

    let held = forgotten(1, "/t/old.fake", b"x|");
    let body = with_id(1, "rest|");
    let files = memory(&[("/t/new.fake", body.as_str())], settled());
    let mut entries = gone(FakeEntries::new(), &held);
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(entries.meta_calls.get(), 1);

    let before = memory(&[("/t/a.fake", "x|")], settled());
    let after = Memory::new();
    let files = Turning::new(before, after, Call::Node, "/t/a.fake");
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(*entries.asked.borrow(), vec!["forgotten:".to_string()]);
}

#[test]
fn a_label_that_holds_an_entry_is_that_entrys_whatever_was_forgotten_there() {
    let held = forgotten(1, "/t/a.fake", b"x|");
    let entries = gone(FakeEntries::new(), &held);
    let mut entries = entries.kept_under(Some("file:/t/a.fake"), id(2), "fake", Tags::default());
    let files = memory(&[("/t/a.fake", "x|y|")], settled());

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    let item = one(&output);
    assert_eq!(item.forgotten, None);
    assert_eq!(item.outcome, Outcome::New(id(2)));
    assert_eq!(
        entries.seen.last().expect("capture").label,
        "file:/t/a.fake"
    );
    assert!(
        !entries
            .asked
            .borrow()
            .iter()
            .any(|text| text == "forgotten:")
    );
}

#[test]
fn two_entries_forgotten_from_one_path_each_guard_what_they_held() {
    let first = forgotten(1, "/t/a.fake", b"x|");
    let second = forgotten(2, "/t/a.fake", b"y|");
    let mut entries = gone(gone(FakeEntries::new(), &first), &second);

    let files = memory(&[("/t/a.fake", "x|z|")], settled());
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(one(&output).outcome, Outcome::Unchanged(fid(1)));
    assert_eq!(one(&output).forgotten, own(1, 0));

    let files = memory(&[("/t/a.fake", "y|zzz|")], settled());
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(one(&output).outcome, Outcome::Unchanged(fid(2)));
    assert_eq!(one(&output).forgotten, own(2, 0));
}

#[test]
fn a_note_renamed_onto_a_forgotten_notes_name_is_moved_there() {
    let body = format!("id={}|renamed|", id(2));
    let files = memory(&[("/t/a.fake", body.as_str())], settled());
    let mut entries = FakeEntries::new()
        .belonging()
        .kept_under(Some("file:/t/a.fake"), id(1), "fake", Tags::default())
        .kept_under(Some("file:/t/b.fake"), id(2), "fake", Tags::default());
    entries
        .heads
        .get_mut("file:/t/a.fake")
        .expect("head")
        .records = 0;
    entries
        .heads
        .get_mut("file:/t/a.fake")
        .expect("head")
        .generation = Generation(2);

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("control");
    assert!(
        matches!(&one(&output).outcome, Outcome::Refused(reason)
            if reason.contains("but this path was kept as entry")),
        "{:?}",
        one(&output).outcome
    );
    assert!(one(&output).moved.is_none());

    let asking = Forget {
        targets: vec![Target::Entry(id(1))],
        listed: 5,
    };
    forget(&asking, &|_| Vec::new(), &mut entries).expect("forgotten");
    entries.writes.clear();
    entries.seen.clear();

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("renamed");

    let item = one(&output);
    assert_eq!(item.moved.as_ref().map(|moved| moved.id), Some(id(2)));
    assert_eq!(item.outcome, Outcome::New(id(2)));
    let seen = entries.seen.last().expect("a capture");
    assert_eq!(seen.label, "file:/t/a.fake");
    assert_eq!(seen.bytes, body.as_bytes());
    assert_eq!(item.forgotten, None);
    assert!(
        entries
            .writes
            .contains(&Write::Meta(id(2), Some(label("/t/a.fake"))))
    );
}

#[test]
fn a_file_passed_over_counts_as_seen_and_the_gone_rule_leaves_a_forgotten_entry_alone() {
    let own_entry = forgotten(1, "/t/a.fake", b"x|");
    let far = forgotten(2, "/t/far.fake", b"y|");
    let half = forgotten(4, "/t/half.fake", b"x|y|");
    let files = memory(&[("/t/a.fake", "x|z|")], settled());
    let mut entries = gone(gone(FakeEntries::new(), &own_entry), &far);
    entries = forgotten_entry(entries, "fake", &half, (1, 2));
    entries = entries.kept_under(Some("file:/t/gone.fake"), id(3), "fake", Tags::default());

    pass_of(FakeKind::promising(), &files, &mut entries, &["/t"]);

    assert_eq!(bare_captures(&entries).len(), 2);
    assert!(bare_captures(&entries).contains(&"file:/t/gone.fake"));
    assert!(bare_captures(&entries).contains(&own_entry.label().as_str()));
    assert!(
        !entries
            .seen
            .iter()
            .any(|seen| seen.label == far.label().to_string())
    );
    assert!(metas(&entries).is_empty());
    assert!(
        !entries
            .seen
            .iter()
            .any(|seen| seen.label == half.label().to_string())
    );
    let kept_half = entries.heads[half.label().as_str()];
    assert_eq!(
        (kept_half.generation, kept_half.records),
        (Generation(1), 2)
    );
    assert_eq!(
        *entries.asked.borrow(),
        vec!["forgotten:".to_string(), "/t".to_string()]
    );
    assert_eq!(
        entries.heads[own_entry.label().as_str()].generation,
        Generation(2)
    );
}

#[test]
fn a_stamp_stands_for_the_label_its_entry_had() {
    let held = forgotten(1, "/t/old.fake", b"x|");
    let mut entries = gone(FakeEntries::new(), &held);
    let body = with_id(1, "rest|");
    let first = Counting::new(memory(&[("/t/new.fake", body.as_str())], settled()));

    let output = run(FakeKind::new(), &first, &mut entries, &request("/t")).expect("first");

    assert_eq!(one(&output).forgotten, own(1, ADOPTED));
    let moved = Forgotten {
        had: label("/t/new.fake"),
        ..held
    };
    assert_eq!(entries.metas[&fid(1)].labels.capture, Some(moved.label()));
    assert!(entries.heads[moved.label().as_str()].stamp.is_some());

    let other = with_id(1, "rest|").replace("rest", "else");
    assert_eq!(other.len(), body.len());
    let second = Counting::new(memory(
        &[
            ("/t/new.fake", body.as_str()),
            ("/t/old.fake", other.as_str().replace("id=", "no=").as_str()),
        ],
        settled(),
    ));
    let output = run(FakeKind::new(), &second, &mut entries, &request("/t")).expect("second");

    let old = output
        .items
        .iter()
        .find(|item| item.path.ends_with("old.fake"))
        .expect("old");
    assert!(matches!(old.outcome, Outcome::New(_)));
    assert_eq!(second.read_paths.borrow().len(), 1);

    let held = forgotten(1, "/t/old.fake", b"x|");
    let mut entries = gone(FakeEntries::new(), &held);
    let files = memory(
        &[("/t/new.fake", body.as_str()), ("/t/old.fake", "x|more|")],
        settled(),
    );
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("one keep");
    let at = |name: &str| {
        output
            .items
            .iter()
            .find(|item| item.path.ends_with(name))
            .expect(name)
    };
    assert_eq!(at("new.fake").forgotten, own(1, ADOPTED));
    assert!(matches!(at("old.fake").outcome, Outcome::New(new) if new != fid(1)));
    assert_eq!(at("old.fake").forgotten, None);
}

#[test]
fn an_absent_stamp_matches_nothing() {
    let held = forgotten(1, "/t/a.fake", b"x|");
    let before = Memory::new().unreadable("/t/a.fake", bilbo_entries::files::Node::File);
    let after = memory(&[("/t/a.fake", "x|y|")], settled());
    let files = Turning::new(before, after, Call::Stamp, "/t/a.fake");
    let mut entries = gone(FakeEntries::new(), &held);
    let output = one_pass(&files, &mut entries, "/t/a.fake");
    assert_eq!(one(&output).forgotten, own(1, 0));

    let mut entries = gone(FakeEntries::new(), &held);
    entries
        .heads
        .get_mut(held.label().as_str())
        .expect("head")
        .stamp = Some(Stamp {
        size: 4,
        modified: written(),
    });
    let before = Memory::new().unreadable("/t/a.fake", bilbo_entries::files::Node::File);
    let after = memory(&[("/t/a.fake", "x|y|")], settled());
    let files = Turning::new(before, after, Call::Stamp, "/t/a.fake");
    let output = one_pass(&files, &mut entries, "/t/a.fake");
    assert_eq!(one(&output).forgotten, own(1, 0));
}

fn follows_to(
    other: Option<(&str, &str)>,
    folder_there: bool,
    spelled: bool,
    newest: (u32, u64),
) -> (Output, FakeEntries, Forgotten) {
    let held = forgotten(1, "/t/old.fake", b"x|");
    let body = with_id(1, "rest|");
    let mut files = memory(&[("/t/new.fake", body.as_str())], settled());
    if let Some((path, bytes)) = other {
        files = files.file(path, bytes).modified(path, written());
    }
    if folder_there {
        files = files.folder("/t/old.fake");
    }
    if spelled {
        files = files.alias("/t/old.fake", "/t/new.fake");
    }
    let mut entries = forgotten_entry(FakeEntries::new(), "fake", &held, newest);
    let output = one_pass(&files, &mut entries, "/t/new.fake");
    (output, entries, held)
}

#[test]
fn a_forgotten_entry_follows_its_id_to_where_its_file_is() {
    let cases = [
        ("nothing there", None, false, false),
        ("a folder there", None, true, false),
        ("this very file under another spelling", None, false, true),
        (
            "a file with no id",
            Some(("/t/old.fake", "no id|")),
            false,
            false,
        ),
        (
            "a file with another id",
            Some(("/t/old.fake", "id=01JC5Q3V7XKM4P8R2T6W9YB0DE|")),
            false,
            false,
        ),
    ];
    for (name, other, folder_there, spelled) in cases {
        let (output, entries, held) = follows_to(other, folder_there, spelled, (2, 0));
        let item = output
            .items
            .iter()
            .find(|item| item.path.ends_with("new.fake"))
            .unwrap_or_else(|| panic!("{name}: {:?}", output.items));
        assert_eq!(item.forgotten, own(1, ADOPTED), "{name}");
        let moved = Forgotten {
            had: label("/t/new.fake"),
            ..held.clone()
        };
        assert_eq!(
            entries
                .writes
                .iter()
                .filter(|write| matches!(write, Write::Meta(..)))
                .collect::<Vec<_>>(),
            vec![&Write::Meta(fid(1), Some(moved.label()))],
            "{name}"
        );
        assert_eq!(output.stored_bytes, ADOPTED, "{name}");
        assert!(
            entries.heads[moved.label().as_str()].stamp.is_some(),
            "{name}"
        );
    }
    let (output, entries, held) = follows_to(None, false, false, (1, 1));
    let moved = Forgotten {
        had: label("/t/new.fake"),
        ..held
    };
    assert_eq!(one(&output).forgotten, own(1, ADOPTED));
    assert!(entries.metas[&fid(1)].labels.capture == Some(moved.label()));
    assert!(entries.seen.is_empty());
}

#[test]
fn a_second_file_with_a_forgotten_entrys_id_is_passed_over_and_the_label_stays() {
    let held = forgotten(1, "/t/old.fake", b"x|");
    let first = with_id(1, "a|");
    let second = with_id(1, "b|");
    let files = memory(
        &[
            ("/t/old.fake", first.as_str()),
            ("/t/new.fake", second.as_str()),
        ],
        settled(),
    );
    let mut entries = gone(FakeEntries::new(), &held);
    let counting = Counting::new(files);
    for _ in 0..2 {
        let output = one_pass(&counting, &mut entries, "/t/new.fake");
        assert_eq!(one(&output).forgotten, own(1, 0));
        assert!(entries.writes.is_empty());
        assert!(entries.seen.is_empty());
    }

    let held = forgotten(1, "/t/old.fake", b"x|");
    let files = Memory::new()
        .denied("/t/old.fake")
        .file("/t/new.fake", second.as_bytes())
        .modified("/t/new.fake", written())
        .clock(settled());
    let mut entries = gone(FakeEntries::new(), &held);
    for _ in 0..2 {
        let output = one_pass(&files, &mut entries, "/t/new.fake");
        assert_eq!(one(&output).forgotten, own(1, 0));
        assert!(entries.writes.is_empty());
        assert!(entries.seen.is_empty());
    }
}

fn again(n: u8) -> Option<Met> {
    Some(Met::Again {
        id: fid(n),
        bytes: ADOPTED,
    })
}

fn named(files: &dyn Files, entries: &mut FakeEntries, path: &str) -> Output {
    run(FakeKind::new(), files, entries, &request(path)).expect("run")
}

#[test]
fn a_forgotten_file_named_by_hand_brings_its_entry_back() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Appended);
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());

    let output = named(&files, &mut entries, "/t/a.fake");

    let item = one(&output);
    assert_eq!(item.outcome, Outcome::Appended(fid(1)));
    assert_eq!(item.forgotten, again(1));
    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(fid(1), Some(label("/t/a.fake"))),
            Write::Capture("file:/t/a.fake".to_owned())
        ]
    );
    assert_eq!(output.stored_bytes, ADOPTED + 6 + 100);
    let seen = entries.seen.last().expect("a capture");
    assert_eq!(seen.label, "file:/t/a.fake");
    assert_eq!(seen.bytes, b"x|y|z|");
    assert_eq!(seen.ends, vec![2, 4, 6]);
    assert_eq!(
        entries.metas[&fid(1)].labels.capture,
        Some(label("/t/a.fake"))
    );
}

#[test]
fn a_folder_given_by_hand_passes_a_forgotten_file_over() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held);
    let files = memory(&[("/t/a.fake", "x|y|")], settled());

    let output = named(&files, &mut entries, "/t");

    assert_eq!(one(&output).outcome, Outcome::Unchanged(fid(1)));
    assert_eq!(one(&output).forgotten, own(1, 0));
    assert!(metas(&entries).is_empty());
}

#[test]
fn a_follow_of_one_file_never_brings_a_forgotten_entry_back() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held);
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());

    let output = one_pass(&files, &mut entries, "/t/a.fake");

    assert_eq!(one(&output).outcome, Outcome::Unchanged(fid(1)));
    assert_eq!(one(&output).forgotten, own(1, 0));
    assert!(metas(&entries).is_empty());
}

#[test]
fn a_file_named_by_hand_is_read_whatever_stamp_is_held() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Appended);
    entries
        .heads
        .get_mut(held.label().as_str())
        .expect("head")
        .stamp = Some(Stamp {
        size: 4,
        modified: written(),
    });
    let counting = Counting::new(memory(&[("/t/a.fake", "x|y|")], settled()));

    let output = named(&counting, &mut entries, "/t/a.fake");

    assert_eq!(counting.read_paths.borrow().len(), 1);
    assert_eq!(one(&output).forgotten, again(1));
}

#[test]
fn a_different_file_named_by_hand_is_a_new_entry() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held);
    let files = memory(&[("/t/a.fake", "q|r|")], settled());

    let output = named(&files, &mut entries, "/t/a.fake");

    assert!(matches!(one(&output).outcome, Outcome::New(new) if new != fid(1)));
    assert_eq!(one(&output).forgotten, None);
    assert_eq!(entries.metas[&fid(1)].labels.capture, Some(held.label()));
    assert!(metas(&entries).is_empty());
}

#[test]
fn a_file_brought_back_by_its_id_takes_the_label_of_where_it_is() {
    let held = forgotten(1, "/t/old.fake", b"x|");
    let body = with_id(1, "rest|");
    let mut entries = gone(FakeEntries::new(), &held).on("file:/t/new.fake", Script::Appended);
    let files = memory(&[("/t/new.fake", body.as_str())], settled());

    let output = named(&files, &mut entries, "/t/new.fake");

    assert_eq!(one(&output).forgotten, again(1));
    assert_eq!(
        metas(&entries),
        vec![&Write::Meta(fid(1), Some(label("/t/new.fake")))]
    );
    assert_eq!(
        entries.metas[&fid(1)].labels.capture,
        Some(label("/t/new.fake"))
    );
}

#[test]
fn a_tag_past_the_cap_refuses_the_way_back_before_anything_is_written() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Appended);
    entries.metas.get_mut(&fid(1)).expect("meta").labels.tags = carried(&[("b", by_person())]);
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());
    let cap = "its entry would carry more than tag.cap tags (1)".to_owned();

    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t/a.fake", &["c"], 1),
    )
    .expect("run");

    assert_eq!(one(&output).outcome, Outcome::Refused(cap));
    assert_eq!(one(&output).forgotten, None);
    assert!(entries.writes.is_empty());
    let pass = one_pass(&files, &mut entries, "/t/a.fake");
    assert_eq!(one(&pass).forgotten, own(1, 0));

    entries.meta_calls.set(0);
    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t/a.fake", &["b"], 1),
    )
    .expect("run");
    assert_eq!(one(&output).outcome, Outcome::Appended(fid(1)));
    assert_eq!(one(&output).forgotten, again(1));
    assert_eq!(entries.meta_calls.get(), 5);
}

#[test]
fn a_half_forgotten_entry_named_by_hand_is_what_its_bytes_give() {
    let first = with_id(1, "y|z|");
    let held = forgotten(1, "/t/a.fake", first.as_bytes());
    let edited = format!("edited id={}|y|z|", fid(1));
    let fourth = format!("{first}w|");
    let rows: [(&str, String, Script, Outcome); 3] = [
        (
            "the same",
            first.clone(),
            Script::Unchanged,
            Outcome::Unchanged(fid(1)),
        ),
        (
            "a fourth",
            fourth,
            Script::Appended,
            Outcome::Appended(fid(1)),
        ),
        (
            "the first changed",
            edited,
            Script::NewGeneration,
            Outcome::NewGeneration(fid(1)),
        ),
    ];
    for (name, bytes, script, outcome) in rows {
        assert!(bytes.contains(&format!("id={}", fid(1))), "{name}");
        let mut entries = forgotten_entry(
            FakeEntries::new().on("file:/t/a.fake", script),
            "fake",
            &held,
            (1, 3),
        );
        let files = memory(&[("/t/a.fake", bytes.as_str())], settled());
        let output = named(&files, &mut entries, "/t/a.fake");
        let item = one(&output);
        assert_eq!(item.forgotten, again(1), "{name}");
        assert_eq!(item.outcome, outcome, "{name}");
        assert_eq!(
            entries.metas[&fid(1)].labels.capture,
            Some(label("/t/a.fake")),
            "{name}"
        );
    }

    let nothing = forgotten(2, "/t/b.fake", b"");
    let mut entries = forgotten_entry(
        FakeEntries::new().on("file:/t/b.fake", Script::New),
        "fake",
        &nothing,
        (0, 0),
    );
    let body = with_id(2, "z|");
    let files = memory(&[("/t/b.fake", body.as_str())], settled());
    let output = named(&files, &mut entries, "/t/b.fake");
    assert_eq!(one(&output).outcome, Outcome::New(fid(2)));
    assert_eq!(one(&output).forgotten, again(2));
    assert_eq!(output.tally().new, 1);
    assert_eq!(entries.heads["file:/t/b.fake"].generation, Generation(1));
    assert_eq!(
        entries.metas[&fid(2)].labels.capture,
        Some(label("/t/b.fake"))
    );
}

#[test]
fn a_file_named_by_hand_at_a_path_another_entry_holds_brings_nothing_back() {
    let held = forgotten(1, "/t/old.fake", b"x|");
    let entries = gone(FakeEntries::new(), &held);
    let mut entries = entries.kept_under(Some("file:/t/a.fake"), id(2), "fake", Tags::default());
    let body = with_id(1, "rest|");
    let files = memory(&[("/t/a.fake", body.as_str())], settled());

    let output = named(&files, &mut entries, "/t/a.fake");

    assert_eq!(
        one(&output).outcome,
        Outcome::Refused(format!(
            "its id {} is held by {}, but this path was kept as entry {}",
            fid(1),
            held.label(),
            id(2)
        ))
    );
    assert!(entries.writes.is_empty());
}

#[test]
fn a_way_back_stopped_between_its_two_writes_is_finished_by_the_next_run() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Failure);
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());

    let failed = run(FakeKind::new(), &files, &mut entries, &request("/t/a.fake"));

    assert!(
        matches!(&failed, Err(Error::Store(StoreError::Stopped(what))) if what == "Appended"),
        "{failed:?}"
    );
    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(fid(1), Some(label("/t/a.fake"))),
            Write::Capture("file:/t/a.fake".to_owned())
        ]
    );
    assert_eq!(
        entries.metas[&fid(1)].labels.capture,
        Some(label("/t/a.fake"))
    );
    entries.script.clear();
    entries
        .script
        .insert("file:/t/a.fake".to_owned(), Script::Appended);

    let output = one_pass(&files, &mut entries, "/t/a.fake");

    assert_eq!(one(&output).outcome, Outcome::Appended(fid(1)));
    assert_eq!(one(&output).forgotten, None);
}

#[test]
fn a_file_the_store_refuses_after_its_label_came_back_is_refused_and_the_entry_holds_the_path() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries =
        gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Records("too big".to_owned()));
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());

    let output = named(&files, &mut entries, "/t/a.fake");

    let item = one(&output);
    assert_eq!(
        item.outcome,
        Outcome::Refused("its records do not cover it: too big".to_owned())
    );
    assert_eq!(item.forgotten, again(1));
    assert_eq!(output.stored_bytes, ADOPTED);
    assert_eq!(
        entries.metas[&fid(1)].labels.capture,
        Some(label("/t/a.fake"))
    );
    let asked = entries.asked.borrow().len();

    entries.script.clear();
    entries
        .script
        .insert("file:/t/a.fake".to_owned(), Script::Appended);
    let before = entries.seen.len();
    let output = one_pass(&files, &mut entries, "/t/a.fake");

    assert_eq!(one(&output).outcome, Outcome::Appended(fid(1)));
    assert_eq!(one(&output).forgotten, None);
    assert_eq!(entries.asked.borrow().len(), asked);
    assert_eq!(entries.seen.len(), before + 1);
    let seen = entries.seen.last().expect("a second capture");
    assert_eq!(seen.label, "file:/t/a.fake");
    assert_eq!(seen.bytes, b"x|y|z|");
    assert_eq!(entries.heads["file:/t/a.fake"].id, fid(1));
}

#[test]
fn a_stamp_capture_the_store_refuses_is_the_items_reason_and_a_failure_stops_the_run() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let made = held.label().to_string();
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());
    let mut entries = gone(FakeEntries::new(), &held).on(&made, Script::Records("too big".into()));

    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t", &["a", "b"], 1),
    )
    .expect("run");

    assert_eq!(
        one(&output).outcome,
        Outcome::Refused("its records do not cover it: too big".to_owned())
    );

    let mut entries = gone(FakeEntries::new(), &held).on(&made, Script::Failure);
    let failed = run(FakeKind::new(), &files, &mut entries, &request("/t"));
    assert!(
        matches!(&failed, Err(Error::Store(StoreError::Stopped(what))) if what == "Appended"),
        "{failed:?}"
    );
}

fn name_of(prefix: &str, owner: Id) -> CaptureLabel {
    CaptureLabel::new(format!("{prefix}:{owner}")).expect("a label")
}

fn derived(owner: Id) -> Vec<CaptureLabel> {
    vec![name_of("derive", owner), name_of("summary", owner)]
}

fn former(n: u8, had: &CaptureLabel, held: &[u8]) -> Forgotten {
    Forgotten {
        id: fid(n),
        held: Mark::of(held),
        had: had.clone(),
    }
}

fn with_former(entries: FakeEntries, forgotten: &Forgotten, newest: (u32, u64)) -> FakeEntries {
    forgotten_entry(entries, "document", forgotten, newest)
}

fn owner() -> Id {
    fid(1)
}

fn owner_forgotten() -> FakeEntries {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Appended)
}

fn owner_half() -> FakeEntries {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    forgotten_entry(
        FakeEntries::new().on("file:/t/a.fake", Script::Appended),
        "fake",
        &held,
        (1, 2),
    )
}

fn by_hand(
    entries: &mut FakeEntries,
    labelled: Option<&bilbo_entries::forget::Labelled<'_>>,
    request: &bilbo_entries::remember::Request,
) -> Result<Output, Error> {
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());
    let registry = vec![Box::new(FakeKind::new()) as Box<dyn bilbo_entries::kind::Kind>];
    let mut plan = bilbo_entries::remember::plan(request, &registry, &files)?;
    plan.labelled = labelled;
    bilbo_entries::remember::keep(&plan, &files, entries)
}

fn own_label() -> Option<CaptureLabel> {
    Some(label("/t/a.fake"))
}

fn file_capture() -> Write {
    Write::Capture("file:/t/a.fake".to_owned())
}

#[test]
fn the_way_back_gives_the_names_back_before_its_own_writes() {
    let derive = name_of("derive", owner());
    let summary = name_of("summary", owner());
    let build = || {
        let entries = with_former(owner_forgotten(), &former(2, &derive, b""), (2, 0));
        with_former(entries, &former(3, &summary, b""), (2, 0))
    };
    let mut entries = build();
    let output = by_hand(&mut entries, Some(&derived), &request("/t/a.fake")).expect("run");
    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(fid(2), Some(derive.clone())),
            Write::Meta(fid(3), Some(summary.clone())),
            Write::Meta(fid(1), own_label()),
            file_capture(),
        ]
    );
    assert_eq!(
        one(&output).forgotten,
        Some(Met::Again {
            id: owner(),
            bytes: 3 * ADOPTED
        })
    );
    assert_eq!(output.stored_bytes, 3 * ADOPTED + 6 + 100);

    let mut refused = build();
    refused.metas.get_mut(&fid(1)).expect("meta").labels.tags = carried(&[("b", by_person())]);
    let output = by_hand(
        &mut refused,
        Some(&derived),
        &tagged_request("/t/a.fake", &["c"], 1),
    )
    .expect("run");
    assert!(matches!(one(&output).outcome, Outcome::Refused(_)));
    assert!(refused.writes.is_empty());

    let mut bare = build();
    let output = by_hand(&mut bare, None, &request("/t/a.fake")).expect("run");
    assert_eq!(
        bare.writes,
        vec![Write::Meta(fid(1), own_label()), file_capture()]
    );
    assert_eq!(
        one(&output).forgotten,
        Some(Met::Again {
            id: owner(),
            bytes: ADOPTED
        })
    );
}

#[test]
fn each_name_comes_back_as_it_is_or_not_at_all_and_a_pass_gives_nothing_back() {
    let derive = name_of("derive", owner());
    let summary = name_of("summary", owner());
    let two = b"x|y|".as_slice();
    let run = |entries: &mut FakeEntries| {
        by_hand(entries, Some(&derived), &request("/t/a.fake")).expect("run");
        entries.writes.clone()
    };
    let own = Write::Meta(fid(1), own_label());

    let mut entries = (with_former(
        with_former(owner_forgotten(), &former(2, &derive, b""), (2, 0)),
        &former(3, &summary, b""),
        (2, 0),
    ))
    .kept_under(Some(derive.as_str()), fid(4), "document", Tags::default());
    assert_eq!(
        run(&mut entries),
        vec![
            Write::Meta(fid(3), Some(summary.clone())),
            own.clone(),
            file_capture()
        ]
    );

    let mut entries = with_former(owner_forgotten(), &former(2, &derive, b""), (2, 0));
    assert_eq!(
        run(&mut entries),
        vec![
            Write::Meta(fid(2), Some(derive.clone())),
            own.clone(),
            file_capture()
        ]
    );

    let mut entries =
        owner_half().kept_under(Some(derive.as_str()), fid(4), "document", Tags::default());
    assert_eq!(run(&mut entries), vec![own.clone(), file_capture()]);

    let half = former(2, &derive, two);
    let mut entries = with_former(owner_forgotten(), &half, (1, 2));
    assert_eq!(entries.heads[half.label().as_str()].records, 2);
    assert_eq!(
        run(&mut entries),
        vec![
            Write::Meta(fid(2), Some(derive.clone())),
            own.clone(),
            file_capture()
        ]
    );
    assert_eq!(entries.heads[derive.as_str()].records, 2);
    assert_eq!(entries.heads[derive.as_str()].mark, Mark::of(two));

    let bare = former(2, &derive, b"");
    let mut entries = with_former(owner_forgotten(), &bare, (0, 0));
    assert_eq!(
        run(&mut entries),
        vec![
            Write::Meta(fid(2), Some(derive.clone())),
            own.clone(),
            file_capture()
        ]
    );
    assert_eq!(entries.heads[derive.as_str()].generation, Generation(0));
    assert_eq!(entries.heads[derive.as_str()].records, 0);

    for (older, newer) in [((2, 0), (1, 2)), ((1, 2), (2, 0))] {
        let held = |generation: (u32, u64)| {
            if generation.1 > 0 {
                two
            } else {
                b"".as_slice()
            }
        };
        let mut entries = with_former(
            with_former(owner_forgotten(), &former(2, &derive, held(older)), older),
            &former(5, &derive, held(newer)),
            newer,
        );
        assert_eq!(
            run(&mut entries),
            vec![
                Write::Meta(fid(5), Some(derive.clone())),
                own.clone(),
                file_capture()
            ],
            "{older:?} {newer:?}"
        );
    }

    let mut entries = with_former(
        with_former(owner_forgotten(), &former(2, &derive, b""), (2, 0)),
        &former(3, &summary, b""),
        (2, 0),
    );
    let files = memory(&[("/t/a.fake", "x|y|z|")], settled());
    let pass = pass_of(FakeKind::new(), &files, &mut entries, &["/t/a.fake"]);
    assert_eq!(one(&pass[0]).forgotten, own_met());
    assert_eq!(
        entries.writes,
        vec![Write::Capture(
            forgotten(1, "/t/a.fake", b"x|y|").label().to_string()
        )]
    );
}

fn own_met() -> Option<Met> {
    own(1, 0)
}

#[test]
fn a_way_back_stopped_after_the_names_is_finished_by_the_same_command() {
    let derive = name_of("derive", owner());
    let summary = name_of("summary", owner());
    let mut entries = with_former(
        with_former(owner_forgotten(), &former(2, &derive, b""), (2, 0)),
        &former(3, &summary, b""),
        (2, 0),
    );
    entries.fail_write = Some(3);

    let failed = by_hand(&mut entries, Some(&derived), &request("/t/a.fake"));

    assert!(
        matches!(&failed, Err(Error::Store(StoreError::Stopped(what))) if what == "set_meta"),
        "{failed:?}"
    );
    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(fid(2), Some(derive.clone())),
            Write::Meta(fid(3), Some(summary.clone())),
        ]
    );
    assert_eq!(
        entries.metas[&fid(1)].labels.capture,
        Some(forgotten(1, "/t/a.fake", b"x|y|").label())
    );

    by_hand(&mut entries, Some(&derived), &request("/t/a.fake")).expect("run");

    assert_eq!(
        entries.writes[2..],
        [Write::Meta(fid(1), own_label()), file_capture()]
    );
}

#[test]
fn a_forgotten_file_named_through_the_claims_other_file_comes_back_by_hand() {
    let held = forgotten(1, "/t/a.fake", b"x|y|");
    let mut entries = gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Appended);
    let files = memory(&[("/t/a.fake.z", "zx|y|z|")], settled());

    let output = named(&files, &mut entries, "/t/a.fake.z");

    let item = one(&output);
    assert_eq!(item.outcome, Outcome::Appended(fid(1)));
    assert_eq!(item.forgotten, again(1));
    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(fid(1), Some(label("/t/a.fake"))),
            Write::Capture("file:/t/a.fake".to_owned())
        ]
    );

    let mut bare = gone(FakeEntries::new(), &held).on("file:/t/a.fake", Script::Appended);
    let pass = pass_of(FakeKind::new(), &files, &mut bare, &["/t/a.fake.z"]);
    assert_eq!(one(&pass[0]).forgotten, own(1, 0));
}

#[test]
fn a_lookup_that_fails_before_the_way_back_stops_it_before_any_write() {
    let derive = name_of("derive", owner());
    let mut entries = with_former(owner_forgotten(), &former(2, &derive, b""), (2, 0));
    entries.fail_capture_of = Some(name_of("summary", owner()).to_string());

    let failed = by_hand(&mut entries, Some(&derived), &request("/t/a.fake"));

    assert!(
        matches!(&failed, Err(Error::Store(StoreError::Stopped(what))) if what == "by_capture"),
        "{failed:?}"
    );
    assert!(entries.writes.is_empty());
}
