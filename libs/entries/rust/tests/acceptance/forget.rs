use std::time::{Duration, SystemTime};

use bilbo_entries::forget::{
    Error, Forgotten, Gone, Named, Output, Request, forget as use_case, held,
};
use bilbo_entries::read::refs::Target;
use bilbo_store::entries::{
    Captured, Generation, Head, Kept, Mark, RecordRange, Stamp, StoreError,
};
use bilbo_store::id::Id;
use bilbo_store::labels::{By, CaptureLabel, Standing, Tag, Tags};

use crate::fakes::{FakeEntries, Script, Write, id};

fn forgotten(n: u8, end: u64, had: &str) -> Forgotten {
    Forgotten {
        id: id(n),
        held: Mark { end, hash: 42 },
        had: CaptureLabel::new(had.to_string()).expect("label"),
    }
}

#[test]
fn the_forgotten_entries_are_the_labels_that_read_as_one() {
    let two = forgotten(5, 10, "file:/home/me/notes/shed.md").label();
    let one = forgotten(2, 0, "note:01JC5Q3V7XKM4P8R2T6W9YB0DE").label();
    let mut entries = FakeEntries::new().resolving().belonging();
    entries = entries.kept_under(Some(two.as_str()), id(5), "markdown", Tags::default());
    entries = entries.kept_under(Some(one.as_str()), id(2), "markdown", Tags::default());
    entries = entries.kept_under(
        Some("file:/home/me/forgotten:/shed.md"),
        id(3),
        "markdown",
        Tags::default(),
    );
    entries = entries.kept_under(
        Some("forgotten:not-the-form"),
        id(4),
        "markdown",
        Tags::default(),
    );
    entries = entries.kept_under(
        Some("file:/home/me/notes/garden.md"),
        id(6),
        "markdown",
        Tags::default(),
    );

    let found = held(&entries).expect("held");

    assert_eq!(
        found,
        vec![
            forgotten(2, 0, "note:01JC5Q3V7XKM4P8R2T6W9YB0DE"),
            forgotten(5, 10, "file:/home/me/notes/shed.md"),
        ]
    );
    assert_eq!(*entries.asked.borrow(), vec!["forgotten:".to_string()]);
    assert!(entries.looked.borrow().is_empty());
}

#[test]
fn a_forgotten_label_that_names_another_entry_is_none() {
    let made = forgotten(5, 10, "file:/home/me/notes/shed.md").label();
    assert_eq!(Forgotten::read(id(6), &made), None);

    let entries = FakeEntries::new().resolving().belonging().kept_under(
        Some(made.as_str()),
        id(6),
        "markdown",
        Tags::default(),
    );
    assert_eq!(held(&entries).expect("held"), Vec::new());
}

type Held = Option<(u32, u64, Mark)>;

const SHED: &str = "file:/home/me/notes/shed.md";

fn label(text: &str) -> CaptureLabel {
    CaptureLabel::new(text.to_string()).expect("label")
}

fn head(n: u8, generation: u32, records: u64, mark: Mark) -> Head {
    Head {
        id: id(n),
        generation: Generation(generation),
        records,
        mark,
        stamp: None,
    }
}

fn carried() -> Tags {
    let mut tags = Tags::default();
    tags.set(
        Tag::new("bed").expect("tag"),
        Some(Standing::Carried(By::Person)),
    );
    tags
}

fn holding(mut entries: FakeEntries, n: u8, capture: &str, head_of: Held) -> FakeEntries {
    entries = entries.held(id(n), "markdown", carried());
    let meta = entries.metas.get_mut(&id(n)).expect("meta");
    meta.labels.capture = Some(label(capture));
    meta.labels.id = Some(id(99));
    meta.captured = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
    if let Some((generation, records, mark)) = head_of {
        entries
            .heads
            .insert(capture.to_string(), head(n, generation, records, mark));
    }
    entries
}

fn forget(request: &Request, entries: &mut FakeEntries) -> Result<Output, Error> {
    use_case(request, &|_| Vec::new(), entries)
}

fn asking(targets: Vec<Target>) -> Request {
    Request { targets, listed: 5 }
}

fn entry(n: u8) -> Target {
    Target::Entry(id(n))
}

fn named(output: &Output) -> Vec<(Id, bool, Option<Generation>)> {
    output
        .named
        .iter()
        .map(|named| (named.gone.id, named.gone.forgot, named.gone.held))
        .collect()
}

fn forgotten_label(n: u8, mark: Mark, had: &str) -> CaptureLabel {
    Forgotten {
        id: id(n),
        held: mark,
        had: label(had),
    }
    .label()
}

fn twin(n: u8) -> Id {
    let mut bytes = [n; 16];
    bytes[0] = 0x02;
    Id::from_bytes(bytes)
}

#[test]
fn forgetting_an_entry_writes_its_label_and_then_an_empty_generation() {
    let mark = Mark::of(b"three\nrecords\nhere\n");
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((1, 3, mark)),
    );
    entries.heads.get_mut(SHED).expect("head").stamp = Some(Stamp {
        size: 19,
        modified: SystemTime::UNIX_EPOCH + Duration::from_secs(7),
    });
    let new = forgotten_label(1, mark, SHED);

    let output = forget(&asking(vec![entry(1)]), &mut entries).expect("forgotten");

    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(id(1), Some(new.clone())),
            Write::Capture(new.to_string())
        ]
    );
    let meta = entries.metas.get(&id(1)).expect("meta");
    assert_eq!(meta.labels.capture, Some(new.clone()));
    assert_eq!(meta.labels.id, Some(id(99)));
    assert_eq!(meta.labels.tags, carried());
    assert_eq!(meta.kind, "markdown");
    assert_eq!(
        meta.captured,
        SystemTime::UNIX_EPOCH + Duration::from_secs(5)
    );
    let seen = entries.seen.last().expect("capture");
    assert!(seen.bytes.is_empty() && seen.ends.is_empty());
    assert_eq!(seen.stamp, None);
    let after = entries.heads.get(new.as_str()).expect("head");
    assert_eq!(
        (after.generation, after.records, after.mark, after.stamp),
        (Generation(2), 0, Mark::of(b""), None)
    );
    assert_eq!(
        output,
        Output {
            named: vec![Named {
                gone: Gone {
                    id: id(1),
                    forgot: true,
                    held: Some(Generation(1)),
                    kind: "markdown".into(),
                    had: label(SHED),
                },
                with: Vec::new(),
            }]
        }
    );
}

#[test]
fn an_entry_that_held_nothing_gets_its_label_and_no_generation() {
    let nothing = Mark::of(b"");
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((2, 0, nothing)),
    );
    entries = holding(
        entries,
        2,
        "file:/home/me/notes/shelf.md",
        Some((1, 0, nothing)),
    );
    let first = forgotten_label(1, nothing, SHED);

    let output = forget(&asking(vec![entry(1), entry(2)]), &mut entries).expect("forgotten");

    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(id(1), Some(first.clone())),
            Write::Capture(first.to_string()),
            Write::Meta(
                id(2),
                Some(forgotten_label(2, nothing, "file:/home/me/notes/shelf.md"))
            ),
            Write::Capture(forgotten_label(2, nothing, "file:/home/me/notes/shelf.md").to_string()),
        ]
    );
    let after = entries.heads.get(first.as_str()).expect("head");
    assert_eq!((after.generation, after.records), (Generation(2), 0));
    assert_eq!(
        named(&output),
        vec![(id(1), true, Some(Generation(1))), (id(2), true, None)]
    );
}

#[test]
fn an_entry_already_forgotten_is_unchanged_and_nothing_is_written() {
    let mark = Mark::of(b"what it held\n");
    let forgotten = forgotten_label(1, mark, SHED);
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        forgotten.as_str(),
        Some((2, 0, Mark::of(b""))),
    );

    let output = forget(&asking(vec![entry(1)]), &mut entries).expect("unchanged");

    assert!(entries.writes.is_empty());
    assert_eq!(named(&output), vec![(id(1), false, Some(Generation(1)))]);
    assert_eq!(output.named[0].gone.had, label(SHED));
}

#[test]
fn a_half_forgotten_entry_gets_its_second_write_alone() {
    let mark = Mark::of(b"what it held\n");
    let forgotten = forgotten_label(1, mark, SHED);
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        forgotten.as_str(),
        Some((1, 3, mark)),
    );

    let output = forget(&asking(vec![entry(1)]), &mut entries).expect("forgotten");

    assert_eq!(entries.writes, vec![Write::Capture(forgotten.to_string())]);
    let seen = entries.seen.last().expect("capture");
    assert!(seen.bytes.is_empty() && seen.ends.is_empty());
    assert_eq!(seen.stamp, None);
    assert_eq!(
        entries.metas.get(&id(1)).expect("meta").labels.capture,
        Some(forgotten)
    );
    assert_eq!(named(&output), vec![(id(1), true, Some(Generation(1)))]);
}

#[test]
fn every_ref_is_resolved_before_anything_is_written() {
    let mark = Mark::of(b"a\n");
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((1, 1, mark)),
    );

    let error = forget(&asking(vec![entry(1), entry(7)]), &mut entries).expect_err("no entry");
    assert!(matches!(error, Error::Ref(_)));
    assert_eq!(error.to_string(), format!("no entry {}", id(7)));
    assert!(entries.writes.is_empty());

    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((1, 1, mark)),
    );
    entries = holding(entries, 3, "file:/home/me/notes/a.md", Some((1, 1, mark)));
    entries = holding(entries, 4, "file:/home/me/notes/b.md", Some((1, 1, mark)));
    let mut twins = entries.metas.remove(&id(4)).expect("meta");
    twins.id = twin(3);
    entries
        .heads
        .get_mut("file:/home/me/notes/b.md")
        .expect("head")
        .id = twin(3);
    entries.metas.insert(twin(3), twins);
    let short = twin(3).short();
    assert!(short.ends(id(3)) && short.ends(twin(3)));
    let error =
        forget(&asking(vec![entry(1), Target::Short(short)]), &mut entries).expect_err("ambiguous");
    assert!(matches!(
        error,
        Error::Ref(bilbo_entries::read::Error::Ambiguous { .. })
    ));
    assert!(entries.writes.is_empty());

    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((1, 1, mark)),
    );
    entries = holding(entries, 3, "file:/home/me/notes/a.md", Some((1, 1, mark)));
    entries.fail_meta = Some(2);
    let error = forget(&asking(vec![entry(1), entry(3)]), &mut entries).expect_err("store");
    assert!(
        matches!(&error, Error::Store { error: StoreError::Stopped(what), named }
            if what == "meta" && named.is_empty()),
        "{error:?}"
    );
    assert!(entries.writes.is_empty());
}

#[test]
fn an_entry_named_twice_is_forgotten_once() {
    let mark = Mark::of(b"a\n");
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((1, 1, mark)),
    );

    let output = forget(
        &asking(vec![entry(1), Target::Short(id(1).short()), entry(1)]),
        &mut entries,
    )
    .expect("forgotten");

    assert_eq!(entries.writes.len(), 2);
    assert_eq!(output.named.len(), 1);
}

#[test]
fn an_entry_with_no_capture_label_is_refused_and_nothing_is_written() {
    let mark = Mark::of(b"a\n");
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((1, 1, mark)),
    );
    entries = entries.held(id(2), "markdown", Tags::default());

    let error = forget(&asking(vec![entry(1), entry(2)]), &mut entries).expect_err("no label");

    assert!(matches!(error, Error::NoLabel(refused) if refused == id(2)));
    assert!(error.to_string().contains(&id(2).to_string()));
    assert!(entries.writes.is_empty());
}

#[test]
fn a_run_stopped_at_the_second_write_is_finished_by_the_next() {
    let mark = Mark::of(b"what it held\n");
    let forgotten = forgotten_label(1, mark, SHED);
    let mut entries = holding(
        FakeEntries::new()
            .resolving()
            .belonging()
            .on(forgotten.as_str(), Script::Failure),
        1,
        SHED,
        Some((1, 3, mark)),
    );

    let error = forget(&asking(vec![entry(1)]), &mut entries).expect_err("stopped");

    assert!(
        matches!(&error, Error::Store { error: StoreError::Stopped(what), named }
            if what == "Appended" && *named == vec![id(1)]),
        "{error:?}"
    );
    assert_eq!(
        entries.metas.get(&id(1)).expect("meta").labels.capture,
        Some(forgotten.clone())
    );
    entries.script.clear();
    entries.writes.clear();

    let output = forget(&asking(vec![entry(1)]), &mut entries).expect("finished");

    assert_eq!(entries.writes, vec![Write::Capture(forgotten.to_string())]);
    assert_eq!(named(&output), vec![(id(1), true, Some(Generation(1)))]);

    let nothing = Mark::of(b"");
    let held_none = forgotten_label(2, nothing, "file:/home/me/notes/shelf.md");
    let mut entries = holding(
        FakeEntries::new()
            .resolving()
            .belonging()
            .on(held_none.as_str(), Script::Failure),
        2,
        "file:/home/me/notes/shelf.md",
        Some((2, 0, nothing)),
    );
    let error = forget(&asking(vec![entry(2)]), &mut entries).expect_err("stopped");
    assert!(
        matches!(&error, Error::Store { error: StoreError::Stopped(what), named }
            if what == "Appended" && *named == vec![id(2)]),
        "{error:?}"
    );
    entries.script.clear();
    entries.writes.clear();

    let output = forget(&asking(vec![entry(2)]), &mut entries).expect("unchanged");

    assert!(entries.writes.is_empty());
    assert_eq!(named(&output), vec![(id(2), false, Some(Generation(1)))]);
}

#[test]
fn the_stores_answer_is_judged_against_the_head_it_started_from() {
    let mark = Mark::of(b"a\nb\nc\n");
    let heads: [(&str, Held, &str); 3] = [
        ("records", Some((1, 3, mark)), "new generation"),
        (
            "an empty generation",
            Some((2, 0, Mark::of(b""))),
            "unchanged",
        ),
        ("no generation", Some((0, 0, Mark::of(b""))), "new"),
    ];
    let answers = [
        ("new", Script::New),
        ("appended", Script::Appended),
        ("new generation", Script::NewGeneration),
        ("unchanged", Script::Unchanged),
    ];
    for (name, head_of, right) in heads {
        let held = head_of.map_or(Mark::of(b""), |(_, _, mark)| mark);
        let forgotten = forgotten_label(1, held, SHED);
        for (answer, script) in &answers {
            let mut entries = holding(
                FakeEntries::new()
                    .resolving()
                    .belonging()
                    .on(forgotten.as_str(), script.clone()),
                1,
                SHED,
                head_of,
            );
            let result = forget(&asking(vec![entry(1)]), &mut entries);
            if answer == &right {
                assert!(result.is_ok(), "{name}: {answer}");
            } else {
                assert!(
                    matches!(&result, Err(Error::NotEmptied { id: refused, named, .. })
                        if *refused == id(1) && *named == vec![id(1)]),
                    "{name}: {answer}: {result:?}"
                );
            }
        }
    }
    let kept_at = |who: u8, generation: u32, records: u64| Kept {
        id: id(who),
        generation: Generation(generation),
        records: RecordRange {
            first: 1,
            count: records,
        },
        raw_bytes: 0,
        stored_bytes: 100,
    };
    let inconsistent = [
        (
            "records",
            Some((1, 3, mark)),
            Captured::NewGeneration(kept_at(9, 2, 0)),
        ),
        (
            "records at the last generation",
            Some((u32::MAX, 3, mark)),
            Captured::NewGeneration(kept_at(1, 0, 0)),
        ),
        (
            "records",
            Some((1, 3, mark)),
            Captured::NewGeneration(kept_at(1, 5, 0)),
        ),
        (
            "records",
            Some((1, 3, mark)),
            Captured::NewGeneration(kept_at(1, 2, 2)),
        ),
        (
            "an empty generation",
            Some((2, 0, Mark::of(b""))),
            Captured::Unchanged(id(9)),
        ),
        (
            "no generation",
            Some((0, 0, Mark::of(b""))),
            Captured::New(kept_at(1, 2, 0)),
        ),
        (
            "no generation",
            Some((0, 0, Mark::of(b""))),
            Captured::New(kept_at(1, 1, 1)),
        ),
    ];
    for (name, head_of, answer) in inconsistent {
        let held = head_of.map_or(Mark::of(b""), |(_, _, mark)| mark);
        let forgotten = forgotten_label(1, held, SHED);
        let mut entries = holding(
            FakeEntries::new()
                .resolving()
                .belonging()
                .on(forgotten.as_str(), Script::Answered(answer)),
            1,
            SHED,
            head_of,
        );
        let result = forget(&asking(vec![entry(1)]), &mut entries);
        assert!(
            matches!(&result, Err(Error::NotEmptied { id: refused, .. }) if *refused == id(1)),
            "{name}: {answer:?}: {result:?}"
        );
    }
}

#[test]
fn an_entry_with_no_generation_gets_its_label_and_its_first_empty_generation() {
    let nothing = Mark::of(b"");
    let forgotten = forgotten_label(1, nothing, SHED);
    let mut entries = holding(
        FakeEntries::new().resolving().belonging(),
        1,
        SHED,
        Some((0, 0, nothing)),
    );

    let output = forget(&asking(vec![entry(1)]), &mut entries).expect("forgotten");

    assert_eq!(
        entries.writes,
        vec![
            Write::Meta(id(1), Some(forgotten.clone())),
            Write::Capture(forgotten.to_string())
        ]
    );
    assert_eq!(named(&output), vec![(id(1), true, None)]);
    let after = entries.heads.get(forgotten.as_str()).expect("head");
    assert_eq!((after.generation, after.records), (Generation(1), 0));

    let mut entries = holding(
        FakeEntries::new()
            .resolving()
            .belonging()
            .on(forgotten.as_str(), Script::Failure),
        1,
        SHED,
        Some((0, 0, nothing)),
    );
    let error = forget(&asking(vec![entry(1)]), &mut entries).expect_err("stopped");
    assert!(matches!(error, Error::Store { .. }));
    entries.script.clear();
    entries.writes.clear();

    let output = forget(&asking(vec![entry(1)]), &mut entries).expect("finished");

    assert_eq!(entries.writes, vec![Write::Capture(forgotten.to_string())]);
    assert_eq!(named(&output), vec![(id(1), true, None)]);
    let after = entries.heads.get(forgotten.as_str()).expect("head");
    assert_eq!((after.generation, after.records), (Generation(1), 0));
}

fn kept(
    entries: FakeEntries,
    n: u8,
    kind: &str,
    capture: &str,
    parent: Option<u8>,
    state: Held,
) -> FakeEntries {
    let mut entries = holding(entries, n, capture, state);
    let meta = entries.metas.get_mut(&id(n)).expect("meta");
    meta.labels.id = parent.map(id);
    meta.kind = kind.to_string();
    entries
}

fn names(id: Id) -> Vec<CaptureLabel> {
    vec![
        label(&format!("derive:{id}")),
        label(&format!("summary:{id}")),
    ]
}

fn use_with_names(request: &Request, entries: &mut FakeEntries) -> Result<Output, Error> {
    use_case(request, &names, entries)
}

fn mark_of(n: u8) -> Mark {
    Mark::of(format!("{n}\n{n}\n{n}\n").as_bytes())
}

fn live(n: u8) -> Held {
    Some((1, 3, mark_of(n)))
}

fn two(n: u8, had: &str) -> Vec<Write> {
    let made = forgotten_label(n, mark_of(n), had);
    vec![
        Write::Meta(id(n), Some(made.clone())),
        Write::Capture(made.to_string()),
    ]
}

fn with_ids(named: &Named) -> Vec<Id> {
    named.with.iter().map(|gone| gone.id).collect()
}

const ONE: &str = "file:/home/me/src/garden/one.jsonl";
const TWO: &str = "file:/home/me/src/garden/two.jsonl";
const THREE: &str = "file:/home/me/src/garden/three.jsonl";

fn derived(n: u8) -> String {
    format!("derive:{}", id(n))
}

fn summarised(n: u8) -> String {
    format!("summary:{}", id(n))
}

fn family() -> FakeEntries {
    let mut entries = FakeEntries::new().resolving().belonging();
    entries = kept(entries, 1, "session", ONE, None, live(1));
    entries = kept(entries, 2, "session", TWO, Some(1), live(2));
    entries = kept(entries, 3, "summary", &summarised(1), None, live(3));
    entries = kept(entries, 4, "session", THREE, Some(2), live(4));
    entries = kept(entries, 5, "note", &derived(1), Some(1), live(5));
    kept(
        entries,
        6,
        "markdown",
        "file:/home/me/notes/shed.md",
        None,
        live(6),
    )
}

#[test]
fn what_belongs_goes_with_it_however_deep() {
    let mut entries = family();

    let output = use_with_names(&asking(vec![entry(1)]), &mut entries).expect("forgotten");

    let mut expected = two(1, ONE);
    expected.extend(two(2, TWO));
    expected.extend(two(3, &summarised(1)));
    expected.extend(two(4, THREE));
    expected.extend(two(5, &derived(1)));
    assert_eq!(entries.writes, expected);
    assert_eq!(with_ids(&output.named[0]), vec![id(2), id(3), id(4), id(5)]);
    assert!(output.named[0].with.iter().all(|gone| gone.forgot));
}

#[test]
fn a_summary_of_one_that_belongs_goes_too() {
    let mut entries = family();
    entries = kept(entries, 7, "summary", &summarised(2), None, live(7));

    let output = use_with_names(&asking(vec![entry(1)]), &mut entries).expect("forgotten");

    assert!(with_ids(&output.named[0]).contains(&id(7)));
    let at = two(7, &summarised(2));
    assert!(entries.writes.windows(2).any(|pair| pair == at.as_slice()));
    let made = forgotten_label(7, mark_of(7), &summarised(2)).to_string();
    let seen = entries
        .seen
        .iter()
        .find(|seen| seen.label == made)
        .expect("a capture");
    assert!(seen.bytes.is_empty() && seen.ends.is_empty());
    assert_eq!(seen.stamp, None);
}

#[test]
fn one_that_belongs_is_forgotten_on_its_own_and_its_parent_stays() {
    let mut entries = family();

    let output = use_with_names(&asking(vec![entry(2)]), &mut entries).expect("forgotten");

    let mut expected = two(2, TWO);
    expected.extend(two(4, THREE));
    assert_eq!(entries.writes, expected);
    assert_eq!(with_ids(&output.named[0]), vec![id(4)]);
}

fn left_by_a_stopped_run(with_current: bool) -> FakeEntries {
    let mut entries = FakeEntries::new().resolving().belonging();
    let parent = forgotten_label(1, mark_of(1), ONE);
    let former = forgotten_label(3, mark_of(3), &summarised(1));
    entries = kept(
        entries,
        1,
        "session",
        parent.as_str(),
        None,
        Some((2, 0, Mark::of(b""))),
    );
    entries = kept(
        entries,
        2,
        "session",
        forgotten_label(2, mark_of(2), TWO).as_str(),
        Some(1),
        Some((2, 0, Mark::of(b""))),
    );
    entries = kept(entries, 3, "summary", former.as_str(), None, live(3));
    entries = kept(entries, 5, "note", &derived(1), Some(1), live(5));
    if with_current {
        entries = kept(entries, 7, "summary", &summarised(1), None, live(7));
    }
    entries
}

#[test]
fn forgetting_again_finishes_what_a_stopped_run_left() {
    let former = forgotten_label(3, mark_of(3), &summarised(1));
    for with_current in [true, false] {
        let mut entries = left_by_a_stopped_run(with_current);

        let output = use_with_names(&asking(vec![entry(1)]), &mut entries).expect("finished");

        let mut expected = vec![Write::Capture(former.to_string())];
        expected.extend(two(5, &derived(1)));
        let mut ids = vec![id(3), id(5)];
        if with_current {
            expected.extend(two(7, &summarised(1)));
            ids.push(id(7));
        }
        assert_eq!(entries.writes, expected, "{with_current}");
        assert!(!output.named[0].gone.forgot);
        assert_eq!(with_ids(&output.named[0]), ids, "{with_current}");
        assert!(output.named[0].with.iter().all(|gone| gone.forgot));
    }
}

#[test]
fn an_entry_named_that_belongs_to_another_named_counts_as_named() {
    let mut entries = family();

    let output =
        use_with_names(&asking(vec![entry(1), entry(2)]), &mut entries).expect("forgotten");

    assert_eq!(
        output
            .named
            .iter()
            .map(|named| named.gone.id)
            .collect::<Vec<_>>(),
        vec![id(1), id(2)]
    );
    assert_eq!(with_ids(&output.named[0]), vec![id(3), id(4), id(5)]);
    assert!(output.named[1].with.is_empty());
    let mut expected = two(1, ONE);
    expected.extend(two(3, &summarised(1)));
    expected.extend(two(4, THREE));
    expected.extend(two(5, &derived(1)));
    expected.extend(two(2, TWO));
    assert_eq!(entries.writes, expected);
}

fn quartet() -> FakeEntries {
    let mut entries = FakeEntries::new().resolving().belonging();
    entries = kept(entries, 1, "session", ONE, None, live(1));
    entries = kept(
        entries,
        6,
        "markdown",
        "file:/home/me/notes/shed.md",
        None,
        live(6),
    );
    entries = kept(entries, 2, "session", TWO, Some(1), live(2));
    kept(entries, 3, "session", THREE, Some(2), live(3))
}

fn stopped_at_a_meta(error: &Error, named: &[Id]) -> bool {
    matches!(error, Error::Store { error: StoreError::Stopped(what), named: ids }
        if what == "set_meta" && ids == named)
}

#[test]
fn named_entries_are_written_in_the_order_given_and_a_shared_one_goes_with_the_first() {
    let shed = "file:/home/me/notes/shed.md";

    let mut entries = quartet();
    let output =
        use_with_names(&asking(vec![entry(1), entry(6), entry(2)]), &mut entries).expect("S U A");
    let mut expected = two(1, ONE);
    expected.extend(two(3, THREE));
    expected.extend(two(6, shed));
    expected.extend(two(2, TWO));
    assert_eq!(entries.writes, expected);
    assert_eq!(with_ids(&output.named[0]), vec![id(3)]);

    let mut entries = quartet();
    let output = use_with_names(&asking(vec![entry(2), entry(1)]), &mut entries).expect("A S");
    let mut expected = two(2, TWO);
    expected.extend(two(3, THREE));
    expected.extend(two(1, ONE));
    assert_eq!(entries.writes, expected);
    assert_eq!(with_ids(&output.named[0]), vec![id(3)]);

    let mut entries = quartet();
    entries.fail_write = Some(5);
    let error = use_with_names(&asking(vec![entry(1), entry(6), entry(2)]), &mut entries)
        .expect_err("stopped");
    assert!(
        stopped_at_a_meta(&error, &[id(1), id(6), id(2)]),
        "{error:?}"
    );
    let mut expected = two(1, ONE);
    expected.extend(two(3, THREE));
    assert_eq!(entries.writes, expected);

    let mut entries = quartet();
    entries.fail_write = Some(5);
    let error =
        use_with_names(&asking(vec![entry(2), entry(1)]), &mut entries).expect_err("stopped");
    assert!(stopped_at_a_meta(&error, &[id(2), id(1)]), "{error:?}");
    let mut expected = two(2, TWO);
    expected.extend(two(3, THREE));
    assert_eq!(entries.writes, expected);
}

#[test]
fn a_lookup_the_store_fails_after_the_list_is_whole_carries_the_named_ids() {
    type Switch = fn(&mut FakeEntries);
    let switches: [(&str, Switch, bool); 6] = [
        (
            "captures_with",
            |entries| entries.fail_captures_with = true,
            false,
        ),
        (
            "belonging_to",
            |entries| entries.fail_belonging = true,
            false,
        ),
        (
            "a head of its own",
            |entries| entries.lookup_fails = true,
            false,
        ),
        (
            "a head of what is gathered",
            |entries| entries.lookup_fails = true,
            true,
        ),
        (
            "a member's meta",
            |entries| entries.fail_meta = Some(2),
            false,
        ),
        (
            "a member's head",
            |entries| entries.fail_capture_of = Some(TWO.to_owned()),
            false,
        ),
    ];
    for (name, switch, with_labels) in switches {
        let mut entries = family();
        switch(&mut entries);
        let request = asking(vec![entry(1)]);

        let error = if with_labels {
            use_with_names(&request, &mut entries)
        } else {
            forget(&request, &mut entries)
        }
        .expect_err(name);

        assert!(
            matches!(&error, Error::Store { named, .. } if *named == vec![id(1)]),
            "{name}: {error:?}"
        );
        assert!(entries.writes.is_empty(), "{name}");
    }
}

#[test]
fn a_capture_the_store_refuses_is_kept_as_the_cause_of_the_error() {
    let mark = Mark::of(b"a\n");
    let forgotten = forgotten_label(1, mark, SHED);
    let mut entries = holding(
        FakeEntries::new()
            .resolving()
            .belonging()
            .on(forgotten.as_str(), Script::Records("too big".to_owned())),
        1,
        SHED,
        Some((1, 1, mark)),
    );

    let error = forget(&asking(vec![entry(1)]), &mut entries).expect_err("refused");

    assert!(
        matches!(&error, Error::NotEmptied { cause: Some(_), .. }),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        format!(
            "entry {} has its forgotten label, and the store did not empty it",
            id(1)
        )
    );
    let cause = std::error::Error::source(&error).expect("a cause");
    assert_eq!(cause.to_string(), "its records do not cover it: too big");
}
