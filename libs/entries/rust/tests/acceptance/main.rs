mod fakes;
mod read;
mod tag;

use std::path::{Path, PathBuf};

use std::time::{Duration, SystemTime};

use bilbo_entries::files::{Call, Files, Memory, Node, Turning};
use bilbo_entries::follows::{self, Follow, Planned};
use bilbo_entries::kind::Kind;
use bilbo_entries::remember::{
    self, Error, Item, Moved, Outcome, Output, Request, Tagging, Tally, Written, WrittenAnswer,
};
use bilbo_store::entries::{Generation, Head, Mark, Stamp};
use bilbo_store::id::Id;
use bilbo_store::labels::{By, CaptureLabel, Standing, StoredLabels, Tag, Tags};
use fakes::{ADOPTED, Counting, FakeEntries, FakeKind, Script, id};

const HELD: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

fn kinds(kind: FakeKind) -> Vec<Box<dyn Kind>> {
    vec![Box::new(kind)]
}

fn request(target: &str) -> Request {
    Request {
        target: PathBuf::from(target),
        kind: None,
        tags: Tagging::default(),
    }
}

fn tagged_request(target: &str, names: &[&str], cap: usize) -> Request {
    Request {
        tags: Tagging {
            tags: names
                .iter()
                .map(|name| Tag::new(name).expect("a tag"))
                .collect(),
            cap,
        },
        ..request(target)
    }
}

fn carried(pairs: &[(&str, Option<Standing>)]) -> Tags {
    let mut tags = Tags::default();
    for (name, standing) in pairs {
        tags.set(Tag::new(name).expect("a tag"), *standing);
    }
    tags
}

fn by_person() -> Option<Standing> {
    Some(Standing::Carried(By::Person))
}

fn run(
    kind: FakeKind,
    files: &dyn Files,
    entries: &mut FakeEntries,
    request: &Request,
) -> Result<Output, Error> {
    let registry = kinds(kind);
    let plan = remember::plan(request, &registry, files)?;
    remember::keep(&plan, files, entries)
}

fn outcomes(output: &Output) -> Vec<(&str, &Outcome)> {
    output
        .items
        .iter()
        .map(|item| (item.path.to_str().expect("utf-8"), &item.outcome))
        .collect()
}

#[test]
fn each_capture_outcome_is_listed_and_counted() {
    let files = Memory::new()
        .file("/t/a.fake", "x|y|")
        .file("/t/b.fake", "x|y|z|")
        .file("/t/c.fake", "x|")
        .file("/t/d.fake", "x|");
    let mut entries = FakeEntries::new()
        .on("file:/t/b.fake", Script::Appended)
        .on("file:/t/c.fake", Script::NewGeneration)
        .on("file:/t/d.fake", Script::Unchanged);
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    assert_eq!(output.claimed, 4);
    assert_eq!(output.kind.as_deref(), Some("fake"));
    assert_eq!(
        outcomes(&output),
        vec![
            ("/t/a.fake", &Outcome::New(id(1))),
            ("/t/b.fake", &Outcome::Appended(id(2))),
            ("/t/c.fake", &Outcome::NewGeneration(id(3))),
            ("/t/d.fake", &Outcome::Unchanged(id(4))),
        ]
    );
    assert_eq!(
        output.tally(),
        Tally {
            new: 1,
            appended: 1,
            new_generation: 1,
            unchanged: 1,
            refused: 0,
        }
    );
}

#[test]
fn records_raw_and_stored_bytes_add_up_over_the_kept_files() {
    let files = Memory::new()
        .file("/t/a.fake", "x|y|")
        .file("/t/b.fake", "x|y|z|")
        .file("/t/c.fake", "unchanged|");
    let mut entries = FakeEntries::new().on("file:/t/c.fake", Script::Unchanged);
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    assert_eq!(output.records, 2 + 3);
    assert_eq!(output.raw_bytes, 4 + 6);
    assert_eq!(output.stored_bytes, (4 + 100) + (6 + 100));
}

#[test]
fn the_kinds_proposal_reaches_the_capture() {
    let body = format!("id={HELD}|rest|");
    let files = Memory::new()
        .file("/t/a.fake", body.as_bytes())
        .file("/t/b.fake", "no id here|");
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    assert_eq!(entries.seen.len(), 2);
    let first = &entries.seen[0];
    assert_eq!(first.label, "file:/t/a.fake");
    assert_eq!(first.kind, "fake");
    assert_eq!(first.id, bilbo_store::id::Id::parse(HELD));
    assert_eq!(first.id_label, None);
    assert_eq!(first.bytes, body.as_bytes());
    assert_eq!(
        first.ends,
        vec![body.find('|').expect("bar") + 1, body.len()]
    );
    assert_eq!(entries.seen[1].id, None);
}

#[test]
fn a_file_is_refused_for_its_reason_and_the_run_goes_on() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let bad = PathBuf::from(OsString::from_vec(b"/t/\xFF.fake".to_vec()));
    let files = Memory::new()
        .unreadable("/t/a-locked.fake", Node::File)
        .file("/t/b-kind.fake", "!no")
        .file("/t/c-held.fake", format!("id={}|", id(1)).as_bytes())
        .file("/t/other.md", format!("id={}|", id(1)).as_bytes())
        .file("/t/d-records.fake", "x|")
        .file(bad.clone(), "x|")
        .file("/t/z-fine.fake", "x|");
    let mut entries = FakeEntries::new()
        .kept_under(Some("file:/t/other.md"), id(1), "fake", Tags::default())
        .on(
            "file:/t/c-held.fake",
            Script::IdHeld(CaptureLabel::new("file:/t/other.md".into())),
        )
        .on(
            "file:/t/d-records.fake",
            Script::Records("end 3 is past the bytes".into()),
        );
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    let reasons: Vec<(PathBuf, String)> = output
        .items
        .iter()
        .filter_map(|item| match &item.outcome {
            Outcome::Refused(reason) => Some((item.path.clone(), reason.clone())),
            _ => None,
        })
        .collect();
    let held = format!("its id {} is held by file:/t/other.md", id(1));
    assert_eq!(
        reasons,
        vec![
            (
                PathBuf::from("/t/a-locked.fake"),
                "cannot read: permission denied".to_string()
            ),
            (
                PathBuf::from("/t/b-kind.fake"),
                "the fake kind refuses it".to_string()
            ),
            (PathBuf::from("/t/c-held.fake"), held),
            (
                PathBuf::from("/t/d-records.fake"),
                "its records do not cover it: end 3 is past the bytes".to_string()
            ),
            (bad, "its path is not UTF-8".to_string()),
        ]
    );
    assert_eq!(output.claimed, 6);
    assert_eq!(output.tally().refused, 5);
    assert_eq!(output.tally().new, 1);
    assert!(output.items.iter().any(|item| matches!(
        item,
        Item {
            path,
            outcome: Outcome::New(_),
            moved: None,
            carried: None,
            withdrew: None,
        } if path == Path::new("/t/z-fine.fake")
    )));
}

#[test]
fn a_folder_the_kind_could_not_read_is_listed_as_refused() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut kind = FakeKind::new();
    kind.refused
        .push((PathBuf::from("/t/locked"), "cannot read: denied".into()));
    let mut entries = FakeEntries::new();
    let output = run(kind, &files, &mut entries, &request("/t")).expect("run");

    assert_eq!(output.claimed, 1);
    assert_eq!(
        outcomes(&output),
        vec![
            (
                "/t/locked",
                &Outcome::Refused("cannot read: denied".to_string())
            ),
            ("/t/a.fake", &Outcome::New(id(1))),
        ]
    );
}

#[test]
fn a_store_failure_stops_the_run() {
    let files = Memory::new()
        .file("/t/a.fake", "x|")
        .file("/t/b.fake", "x|")
        .file("/t/c.fake", "x|");
    let mut entries = FakeEntries::new().on("file:/t/b.fake", Script::Failure);
    let error =
        run(FakeKind::new(), &files, &mut entries, &request("/t")).expect_err("the run stops");

    assert!(matches!(error, Error::Store(_)));
    assert_eq!(error.to_string(), "stopped at the test hook Appended");
    assert_eq!(entries.seen.len(), 2);
}

#[test]
fn files_are_kept_in_the_order_the_kind_claims_them() {
    let files = Memory::new()
        .file("/t/b.fake", "x|")
        .file("/t/a/z.fake", "x|")
        .file("/t/a.fake", "x|");
    let mut entries = FakeEntries::new();
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    let order: Vec<&str> = output
        .items
        .iter()
        .map(|item| item.path.to_str().expect("utf-8"))
        .collect();
    let seen: Vec<&str> = entries.seen.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(order, vec!["/t/a/z.fake", "/t/a.fake", "/t/b.fake"]);
    assert_eq!(
        seen,
        vec!["file:/t/a/z.fake", "file:/t/a.fake", "file:/t/b.fake"]
    );
}

#[test]
fn an_unkept_plan_lists_only_its_refusals_and_counts_nothing() {
    let files = Memory::new().file("/t/a.txt", "x");
    let mut kind = FakeKind::new();
    kind.refused
        .push((PathBuf::from("/t/locked"), "cannot read: denied".into()));
    let registry = kinds(kind);
    let plan = remember::plan(&request("/t"), &registry, &files).expect("plan");
    assert!(plan.kind.is_none());

    let mut entries = FakeEntries::new();
    let output = remember::keep(&plan, &files, &mut entries).expect("keep");

    assert_eq!(output, Output::unkept(&plan));
    assert_eq!(output.claimed, 0);
    assert_eq!(
        (output.records, output.raw_bytes, output.stored_bytes),
        (0, 0, 0)
    );
    assert_eq!(
        outcomes(&output),
        vec![(
            "/t/locked",
            &Outcome::Refused("cannot read: denied".to_string())
        )]
    );
    assert_eq!(output.tally().refused, 1);
    assert!(entries.seen.is_empty());
}

#[test]
fn a_file_whose_size_and_time_did_not_change_is_not_read_again() {
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(60);
    let files = Memory::new()
        .file("/t/a.fake", "x|")
        .file("/t/b.fake", "y|");
    let mut entries = FakeEntries::new();
    let first = Counting::new(files.clone());
    run(FakeKind::new(), &first, &mut entries, &request("/t")).expect("first run");
    assert_eq!(first.read_paths.borrow().len(), 2);

    let again = Counting::new(files.clone());
    let output = run(FakeKind::new(), &again, &mut entries, &request("/t")).expect("again");
    assert_eq!(again.read_paths.borrow().len(), 0);
    assert_eq!(
        outcomes(&output),
        vec![
            ("/t/a.fake", &Outcome::Unchanged(id(1))),
            ("/t/b.fake", &Outcome::Unchanged(id(2))),
        ]
    );
    assert_eq!(entries.seen.len(), 2, "a skipped file reaches no capture");

    let grown = Counting::new(files.clone().file("/t/b.fake", "y|z|"));
    run(FakeKind::new(), &grown, &mut entries, &request("/t")).expect("grown");
    assert_eq!(grown.read_paths.borrow().len(), 1);

    let touched = Counting::new(files.modified("/t/a.fake", later));
    run(FakeKind::new(), &touched, &mut entries, &request("/t")).expect("touched");
    assert_eq!(
        touched.read_paths.borrow().len(),
        2,
        "b changed in the run before"
    );
}

#[test]
fn the_capture_carries_the_stamp_taken_before_the_read() {
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(7);
    let files = Memory::new()
        .file("/t/a.fake", "x|yz|")
        .modified("/t/a.fake", later);
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(
        entries.seen[0].stamp,
        Some(Stamp {
            size: 5,
            modified: later
        })
    );
}

#[test]
fn the_bytes_after_the_last_record_are_left_for_a_later_run() {
    let files = Memory::new().file("/t/a.fake", "x|y|half a rec");
    let mut entries = FakeEntries::new();
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(entries.seen[0].bytes, b"x|y|");
    assert_eq!(entries.seen[0].ends, vec![2, 4]);
    assert_eq!(output.raw_bytes, 4);
}

#[test]
fn a_file_proposes_as_its_id_label_the_entry_kept_for_another() {
    let files = Memory::new()
        .file("/t/a.fake", "main|")
        .file("/t/a.sub.fake", "sub|")
        .file("/t/z.sub.fake", "orphan|");
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    let id_labels: Vec<(&str, Option<bilbo_store::id::Id>)> = entries
        .seen
        .iter()
        .map(|seen| (seen.label.as_str(), seen.id_label))
        .collect();
    assert_eq!(
        id_labels,
        vec![
            ("file:/t/a.fake", None),
            ("file:/t/a.sub.fake", Some(id(1))),
            ("file:/t/z.sub.fake", None),
        ]
    );
}

#[test]
fn a_subagent_kept_before_its_session_gets_its_id_label_when_it_is_next_read() {
    let mut entries = FakeEntries::new();
    let alone = Memory::new().file("/t/a.sub.fake", "sub|");
    run(FakeKind::new(), &alone, &mut entries, &request("/t")).expect("the child alone");
    let child = id(1);
    assert_eq!(entries.metas[&child].labels.id, None);

    let unchanged = Memory::new()
        .file("/t/a.fake", "main|")
        .file("/t/a.sub.fake", "sub|");
    run(FakeKind::new(), &unchanged, &mut entries, &request("/t")).expect("a sweep");
    assert!(
        entries.adopted.is_empty(),
        "a child its stamp skips is not read, so it waits"
    );

    let grown = Memory::new()
        .file("/t/a.fake", "main|")
        .file("/t/a.sub.fake", "sub|more|");
    run(FakeKind::new(), &grown, &mut entries, &request("/t")).expect("a sweep");
    let main = entries.heads["file:/t/a.fake"].id;
    assert_eq!(entries.adopted, vec![(child, Some(main))]);
    assert_eq!(entries.metas[&child].labels.id, Some(main));

    let more = Memory::new()
        .file("/t/a.fake", "main|")
        .file("/t/a.sub.fake", "sub|more|again|");
    run(FakeKind::new(), &more, &mut entries, &request("/t")).expect("a sweep");
    assert_eq!(entries.adopted.len(), 1, "an id label once given is kept");
}

#[test]
fn adopting_a_subagent_keeps_its_capture_label() {
    let mut entries = FakeEntries::new();
    let alone = Memory::new().file("/t/a.sub.fake", "sub|");
    run(FakeKind::new(), &alone, &mut entries, &request("/t")).expect("the child alone");
    let child = id(1);
    let held = carried(&[("a", by_person()), ("b", Some(Standing::Removed))]);
    entries.metas.get_mut(&child).expect("a meta").labels.tags = held.clone();
    let grown = Memory::new()
        .file("/t/a.fake", "main|")
        .file("/t/a.sub.fake", "sub|more|");
    run(FakeKind::new(), &grown, &mut entries, &request("/t")).expect("a sweep");
    let main = entries.heads["file:/t/a.fake"].id;
    assert_eq!(
        entries.metas[&child].labels,
        StoredLabels {
            capture: CaptureLabel::new("file:/t/a.sub.fake".into()),
            id: Some(main),
            tags: held,
        }
    );
}

#[test]
fn a_pass_keeps_each_follow_in_order_and_skips_a_missing_folder() {
    let files = Memory::new()
        .file("/b/one.fake", "x|")
        .file("/a/two.fake", "y|z|");
    let registry = kinds(FakeKind::new());
    let follows = [
        Follow {
            path: "/b".into(),
            kind: None,
        },
        Follow {
            path: "/gone".into(),
            kind: Some("fake".into()),
        },
        Follow {
            path: "/a".into(),
            kind: Some("fake".into()),
        },
    ];
    let planned = follows::plan(&follows, &registry, &files);
    assert!(matches!(&planned[1], Planned::Missing(follow) if follow.path == Path::new("/gone")));
    let mut entries = FakeEntries::new();
    let outputs = follows::keep(&planned, &files, &mut entries).expect("keep");
    let kept: Vec<(&Path, u64)> = outputs
        .iter()
        .map(|output| (output.target.as_path(), output.records))
        .collect();
    assert_eq!(kept, vec![(Path::new("/b"), 1), (Path::new("/a"), 2)]);
    let seen: Vec<&str> = entries.seen.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(seen, vec!["file:/b/one.fake", "file:/a/two.fake"]);
}

#[test]
fn a_file_written_within_two_seconds_of_its_stamp_is_read_again_next_run() {
    let written = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let memory = |clock: u64| {
        Memory::new()
            .file("/t/a.fake", "x|")
            .modified("/t/a.fake", written)
            .clock(written + Duration::from_secs(clock))
    };
    let mut entries = FakeEntries::new();
    let first = Counting::new(memory(1));
    run(FakeKind::new(), &first, &mut entries, &request("/t")).expect("first");
    assert_eq!(entries.seen[0].stamp, None);

    let second = Counting::new(memory(1));
    run(FakeKind::new(), &second, &mut entries, &request("/t")).expect("second");
    assert_eq!(second.read_paths.borrow().len(), 1);

    let settled = Counting::new(memory(3));
    run(FakeKind::new(), &settled, &mut entries, &request("/t")).expect("settled");
    assert_eq!(settled.read_paths.borrow().len(), 1);
    assert_eq!(
        entries.seen.last().expect("a capture").stamp,
        Some(Stamp {
            size: 2,
            modified: written
        })
    );

    let after = Counting::new(memory(3));
    run(FakeKind::new(), &after, &mut entries, &request("/t")).expect("after");
    assert_eq!(after.read_paths.borrow().len(), 0);
}

#[test]
fn a_racy_capture_clears_the_stamp_so_a_look_alike_restore_is_read_again() {
    let written = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let rewritten = written + Duration::from_secs(10);
    let memory = |content: &str, modified: SystemTime, clock: SystemTime| {
        Counting::new(
            Memory::new()
                .file("/t/a.fake", content)
                .modified("/t/a.fake", modified)
                .clock(clock),
        )
    };
    let second = Duration::from_secs;
    let mut entries = FakeEntries::new();
    let settled = memory("x|", written, written + second(3));
    run(FakeKind::new(), &settled, &mut entries, &request("/t")).expect("settled");
    assert!(entries.heads["file:/t/a.fake"].stamp.is_some());

    let racy = memory("y|", rewritten, rewritten + second(1));
    run(FakeKind::new(), &racy, &mut entries, &request("/t")).expect("racy rewrite");
    assert_eq!(entries.heads["file:/t/a.fake"].stamp, None);

    let restored = memory("x|", written, rewritten + second(5));
    run(FakeKind::new(), &restored, &mut entries, &request("/t")).expect("restore");
    assert_eq!(
        restored.read_paths.borrow().len(),
        1,
        "the restore is read, not skipped"
    );
}

#[test]
fn a_kind_whose_last_end_is_past_the_bytes_is_refused() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = FakeEntries::new();
    let output = run(
        FakeKind::overrunning(),
        &files,
        &mut entries,
        &request("/t"),
    )
    .expect("run");
    assert_eq!(
        outcomes(&output),
        vec![(
            "/t/a.fake",
            &Outcome::Refused("its records do not cover it".into())
        )]
    );
    assert!(entries.seen.is_empty());
}

#[test]
fn adopting_an_id_label_counts_its_meta_frame_in_stored_bytes() {
    let mut entries = FakeEntries::new();
    let alone = Memory::new().file("/t/a.sub.fake", "sub|");
    run(FakeKind::new(), &alone, &mut entries, &request("/t")).expect("the child alone");

    let grown = Memory::new()
        .file("/t/a.fake", "main|")
        .file("/t/a.sub.fake", "sub|more|");
    let output = run(FakeKind::new(), &grown, &mut entries, &request("/t")).expect("a sweep");
    let captured: u64 = entries.seen[1..]
        .iter()
        .map(|seen| seen.bytes.len() as u64 + 100)
        .sum();
    assert_eq!(output.stored_bytes, captured + ADOPTED);
}

fn paths(output: &Output) -> Vec<&str> {
    output
        .items
        .iter()
        .map(|item| item.path.to_str().expect("utf-8"))
        .collect()
}

fn refusal(output: &Output) -> &str {
    match &output.items[0].outcome {
        Outcome::Refused(reason) => reason,
        other => panic!("not refused: {other:?}"),
    }
}

fn kept_under(files: &dyn Files, counting: &Counting, entries: &mut FakeEntries) -> Output {
    let registry = kinds(FakeKind::new());
    let plan = remember::plan(&request("/t"), &registry, files).expect("plan");
    remember::keep(&plan, counting, entries).expect("keep")
}

#[test]
fn a_file_already_kept_is_found_by_its_capture_label() {
    let mut entries = FakeEntries::new().on("file:/t/x.fake", Script::Unchanged);
    let plain = Memory::new().file("/t/x.fake", "a|");
    let first = run(FakeKind::new(), &plain, &mut entries, &request("/t")).expect("plain");
    assert_eq!(paths(&first), ["/t/x.fake"]);

    let packed = Memory::new().file("/t/x.fake.z", "za|");
    let second = run(FakeKind::new(), &packed, &mut entries, &request("/t")).expect("packed");
    assert_eq!(paths(&second), ["/t/x.fake.z"]);
    assert_eq!(second.items[0].outcome, Outcome::Unchanged(id(1)));
    assert_eq!(entries.heads.len(), 1);
    assert!(
        entries
            .seen
            .iter()
            .all(|seen| seen.label == "file:/t/x.fake")
    );
}

#[test]
fn the_first_file_that_exists_is_the_one_read() {
    let files = Memory::new()
        .file("/t/x.fake", "a|")
        .file("/t/x.fake.z", "zb|");
    let counting = Counting::new(files.clone());
    let output = kept_under(&files, &counting, &mut FakeEntries::new());
    assert_eq!(paths(&output), ["/t/x.fake"]);
    assert_eq!(*counting.read_paths.borrow(), [PathBuf::from("/t/x.fake")]);
}

#[test]
fn a_file_that_gives_way_to_the_next_between_the_stamp_and_the_read_is_read_there() {
    let files = Turning::new(
        Memory::new().file("/t/x.fake", "a|"),
        Memory::new().file("/t/x.fake.z", "za|"),
        Call::Stamp,
        "/t/x.fake",
    );
    let mut entries = FakeEntries::new();
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(paths(&output), ["/t/x.fake.z"]);
    assert_eq!(output.items[0].outcome, Outcome::New(id(1)));
    assert_eq!(entries.seen[0].bytes, b"a|");
}

#[test]
fn a_file_that_appears_after_it_was_looked_for_is_read_on_the_second_look() {
    let files = Turning::new(
        Memory::new().file("/t/x.fake.z", "za|"),
        Memory::new().file("/t/x.fake", "a|"),
        Call::Node,
        "/t/x.fake",
    );
    let mut entries = FakeEntries::new();
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(paths(&output), ["/t/x.fake"]);
    assert_eq!(output.items[0].outcome, Outcome::New(id(1)));
}

#[test]
fn a_claim_none_of_whose_files_is_there_is_refused_under_its_label() {
    let planned = Memory::new()
        .file("/t/x.fake", "a|")
        .file("/t/y.fake", "b|");
    let counting = Counting::new(Memory::new().file("/t/y.fake", "b|"));
    let output = kept_under(&planned, &counting, &mut FakeEntries::new());
    assert_eq!(paths(&output), ["/t/x.fake", "/t/y.fake"]);
    assert!(refusal(&output).starts_with("cannot read: "));
    assert!(matches!(output.items[1].outcome, Outcome::New(_)));
    assert_eq!(Counting::count(&counting.nodes, "/t/x.fake"), 2);
    assert_eq!(Counting::count(&counting.nodes, "/t/x.fake.z"), 2);
}

#[test]
fn a_file_that_cannot_be_read_is_refused_not_passed_over() {
    let files = Memory::new()
        .unreadable("/t/x.fake", Node::File)
        .file("/t/x.fake.z", "za|");
    let counting = Counting::new(files.clone());
    let output = kept_under(&files, &counting, &mut FakeEntries::new());
    assert_eq!(paths(&output), ["/t/x.fake"]);
    assert_eq!(refusal(&output), "cannot read: permission denied");
    assert_eq!(Counting::count(&counting.read_paths, "/t/x.fake.z"), 0);
}

#[test]
fn a_candidate_that_is_not_a_file_is_never_stamped_or_read() {
    for plain in [
        Memory::new().other("/t/x.fake"),
        Memory::new().folder("/t/x.fake"),
    ] {
        let files = plain.file("/t/x.fake.z", "za|");
        let counting = Counting::new(files.clone());
        let output = kept_under(&files, &counting, &mut FakeEntries::new());
        assert_eq!(paths(&output), ["/t/x.fake.z"]);
        assert!(matches!(output.items[0].outcome, Outcome::New(_)));
        assert_eq!(Counting::count(&counting.stamps, "/t/x.fake"), 0);
        assert_eq!(Counting::count(&counting.read_paths, "/t/x.fake"), 0);
    }
}

#[test]
fn a_claim_with_only_a_non_file_is_refused_as_not_a_regular_file() {
    for node in [
        Memory::new().other("/t/x.fake"),
        Memory::new().folder("/t/x.fake"),
    ] {
        let planned = Memory::new().file("/t/x.fake", "a|");
        let counting = Counting::new(node);
        let output = kept_under(&planned, &counting, &mut FakeEntries::new());
        assert_eq!(paths(&output), ["/t/x.fake"]);
        assert_eq!(refusal(&output), "it is not a regular file");
        assert_eq!(Counting::count(&counting.nodes, "/t/x.fake"), 1);
    }
}

#[test]
fn the_kind_unpacks_what_was_read_before_it_is_split() {
    let files = Memory::new().file("/t/x.fake.z", "za|b|");
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(entries.seen[0].bytes, b"a|b|");
    assert_eq!(entries.seen[0].ends, vec![2, 4]);
}

#[test]
fn a_file_the_kind_cannot_unpack_is_refused_under_its_own_name() {
    let files = Memory::new().file("/t/x.fake.z", "xa|");
    let output = run(
        FakeKind::new(),
        &files,
        &mut FakeEntries::new(),
        &request("/t"),
    )
    .expect("run");
    assert_eq!(paths(&output), ["/t/x.fake.z"]);
    assert_eq!(refusal(&output), "the fake kind cannot unpack it");
}

#[test]
fn the_capture_carries_the_stamp_of_the_file_read() {
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(7);
    let files = Memory::new()
        .file("/t/x.fake.z", "za|")
        .modified("/t/x.fake.z", later);
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    assert_eq!(
        entries.seen[0].stamp,
        Some(Stamp {
            size: 3,
            modified: later
        })
    );
}

#[test]
fn a_stamp_kept_from_the_other_file_does_not_skip_this_one() {
    let mut entries = FakeEntries::new();
    let packed = Memory::new().file("/t/x.fake.z", "za|");
    run(FakeKind::new(), &packed, &mut entries, &request("/t")).expect("packed");
    assert_eq!(entries.seen.len(), 1);

    let both = Memory::new()
        .file("/t/x.fake", "a|b|")
        .file("/t/x.fake.z", "za|");
    let counting = Counting::new(both.clone());
    kept_under(&both, &counting, &mut entries);
    assert_eq!(*counting.read_paths.borrow(), [PathBuf::from("/t/x.fake")]);
    assert_eq!(entries.seen.len(), 2);
}

#[test]
fn a_subagent_gets_its_sessions_id_as_its_id_label() {
    let mut entries = FakeEntries::new();
    run(
        FakeKind::new(),
        &Memory::new().file("/t/a.fake", "x|"),
        &mut entries,
        &request("/t"),
    )
    .expect("session");
    let files = Memory::new()
        .file("/t/a.fake", "x|")
        .file("/t/a.sub.fake.z", "zy|");
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("subagent");
    assert_eq!(entries.seen.last().expect("capture").id_label, Some(id(1)));
}

#[test]
fn a_missing_target_is_planned_when_a_kind_claims_it_through_another_file() {
    for (present, asked) in [("/t/x.fake.z", "/t/x.fake"), ("/t/x.fake", "/t/x.fake.z")] {
        let files = Memory::new().file(present, "za|");
        let registry = kinds(FakeKind::new());
        let plan = remember::plan(&request(asked), &registry, &files).expect("plan");
        assert_eq!(plan.target, PathBuf::from(asked));
        assert_eq!(plan.claims.len(), 1);
        assert_eq!(plan.claims[0].label(), PathBuf::from("/t/x.fake"));
    }
}

#[test]
fn a_missing_target_no_kind_claims_is_the_error_it_was() {
    let files = Memory::new()
        .file("/t/x.fake", "a|")
        .file("/t/other.txt", "b");
    for target in [
        "/t/y.fake",
        "/nope/x.fake",
        "/t/other.md",
        "/nope/deeper/x.fake",
        "/t/..",
    ] {
        for kind in [None, Some("fake")] {
            let registry = kinds(FakeKind::new());
            let request = Request {
                target: PathBuf::from(target),
                kind: kind.map(str::to_owned),
                tags: Tagging::default(),
            };
            match remember::plan(&request, &registry, &files) {
                Err(Error::Target { path, error }) => {
                    assert_eq!(path, PathBuf::from(target));
                    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{target}");
                }
                Err(other) => panic!("{target}: {other}"),
                Ok(_) => panic!("{target} was planned"),
            }
        }
    }
}

#[test]
fn a_followed_file_is_kept_after_it_changes_form() {
    let registry = kinds(FakeKind::new());
    let follows = [Follow {
        path: "/t/x.fake".into(),
        kind: None,
    }];
    let mut entries = FakeEntries::new();

    let plain = Memory::new().file("/t/x.fake", "a|");
    let planned = follows::plan(&follows, &registry, &plain);
    let kept = follows::keep(&planned, &plain, &mut entries).expect("plain");
    assert_eq!(kept[0].items[0].outcome, Outcome::New(id(1)));

    entries
        .script
        .insert("file:/t/x.fake".into(), Script::Unchanged);
    let packed = Memory::new().file("/t/x.fake.z", "za|");
    let planned = follows::plan(&follows, &registry, &packed);
    assert!(matches!(planned[0], Planned::Ready(..)));
    let kept = follows::keep(&planned, &packed, &mut entries).expect("packed");
    assert_eq!(kept[0].items[0].outcome, Outcome::Unchanged(id(1)));

    entries
        .script
        .insert("file:/t/x.fake".into(), Script::Appended);
    let grown = Memory::new().file("/t/x.fake", "a|b|");
    let planned = follows::plan(&follows, &registry, &grown);
    let kept = follows::keep(&planned, &grown, &mut entries).expect("grown");
    assert_eq!(kept[0].items[0].outcome, Outcome::Appended(id(1)));
    assert!(
        entries
            .seen
            .iter()
            .all(|seen| seen.label == "file:/t/x.fake")
    );

    let gone = [Follow {
        path: "/gone".into(),
        kind: None,
    }];
    assert!(matches!(
        follows::plan(&gone, &registry, &grown)[0],
        Planned::Missing(_)
    ));

    let follows = [Follow {
        path: "/t/x.fake.z".into(),
        kind: None,
    }];
    let mut entries = FakeEntries::new();
    let packed = Memory::new().file("/t/x.fake.z", "za|");
    let planned = follows::plan(&follows, &registry, &packed);
    let kept = follows::keep(&planned, &packed, &mut entries).expect("packed");
    assert_eq!(kept[0].items[0].outcome, Outcome::New(id(1)));

    entries
        .script
        .insert("file:/t/x.fake".into(), Script::Appended);
    let grown = Memory::new().file("/t/x.fake", "a|b|");
    let planned = follows::plan(&follows, &registry, &grown);
    assert!(matches!(planned[0], Planned::Ready(..)));
    let kept = follows::keep(&planned, &grown, &mut entries).expect("grown");
    assert_eq!(kept[0].items[0].outcome, Outcome::Appended(id(1)));
    assert_eq!(entries.heads.len(), 1);
    assert!(
        entries
            .seen
            .iter()
            .all(|seen| seen.label == "file:/t/x.fake")
    );
}

#[test]
fn a_file_read_in_its_second_form_is_not_read_again_while_it_is_unchanged() {
    let files = Memory::new().file("/t/x.fake.z", "za|");
    let mut entries = FakeEntries::new();
    let first = Counting::new(files.clone());
    kept_under(&files, &first, &mut entries);
    assert_eq!(first.read_paths.borrow().len(), 1);

    let again = Counting::new(files.clone());
    let output = kept_under(&files, &again, &mut entries);
    assert_eq!(again.read_paths.borrow().len(), 0);
    assert_eq!(output.items[0].outcome, Outcome::Unchanged(id(1)));
    assert_eq!(entries.seen.len(), 1);
}

#[test]
fn a_file_whose_node_cannot_be_read_is_refused_and_no_other_is_tried() {
    let planned = Memory::new()
        .file("/t/x.fake", "a|")
        .file("/t/x.fake.z", "za|");

    let counting = Counting::new(Memory::new().denied("/t/x.fake").file("/t/x.fake.z", "za|"));
    let output = kept_under(&planned, &counting, &mut FakeEntries::new());
    assert_eq!(paths(&output), ["/t/x.fake"]);
    assert_eq!(refusal(&output), "cannot read: permission denied");
    assert_eq!(Counting::count(&counting.nodes, "/t/x.fake.z"), 0);

    let counting = Counting::new(Memory::new().denied("/t/x.fake.z"));
    let output = kept_under(&planned, &counting, &mut FakeEntries::new());
    assert_eq!(paths(&output), ["/t/x.fake.z"]);
    assert_eq!(refusal(&output), "cannot read: permission denied");
    assert_eq!(Counting::count(&counting.nodes, "/t/x.fake"), 1);
}

#[test]
fn a_child_read_from_its_second_file_gets_its_id_label_when_it_is_next_read() {
    let mut entries = FakeEntries::new();
    let alone = Memory::new().file("/t/a.sub.fake.z", "zy|");
    run(FakeKind::new(), &alone, &mut entries, &request("/t")).expect("child alone");
    assert_eq!(entries.metas[&id(1)].labels.id, None);

    let both = Memory::new()
        .file("/t/a.fake", "x|")
        .file("/t/a.sub.fake.z", "zy|z|");
    run(FakeKind::new(), &both, &mut entries, &request("/t")).expect("both");
    assert_eq!(entries.adopted, vec![(id(1), Some(id(2)))]);
}

#[test]
fn remember_with_tags_keeps_a_new_entry_with_them_as_a_persons() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = FakeEntries::new();
    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t", &["a", "b"], 32),
    )
    .expect("run");

    assert_eq!(entries.seen.len(), 1);
    assert!(entries.adopted.is_empty());
    assert_eq!(
        entries.metas[&id(1)].labels.tags,
        carried(&[("a", by_person()), ("b", by_person())])
    );
    assert_eq!(output.tagged, 1);
}

#[test]
fn remember_with_tags_declares_for_a_file_that_did_not_change_and_reads_no_file() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = FakeEntries::new();
    run(
        FakeKind::new(),
        &Counting::new(files.clone()),
        &mut entries,
        &request("/t"),
    )
    .expect("first run");

    let again = Counting::new(files);
    let output = run(
        FakeKind::new(),
        &again,
        &mut entries,
        &tagged_request("/t", &["a"], 32),
    )
    .expect("again");

    assert_eq!(
        outcomes(&output),
        vec![("/t/a.fake", &Outcome::Unchanged(id(1)))]
    );
    assert!(again.read_paths.borrow().is_empty());
    assert_eq!(entries.seen.len(), 1);
    assert_eq!(
        entries.metas[&id(1)].labels.tags,
        carried(&[("a", by_person())])
    );
    assert_eq!(output.tagged, 1);
    assert_eq!(output.stored_bytes, ADOPTED);

    let identical = Counting::new(Memory::new().file("/t/a.fake", "x|"));
    let same = run(
        FakeKind::new(),
        &identical,
        &mut entries,
        &tagged_request("/t", &["a"], 32),
    )
    .expect("identical");
    assert_eq!(same.tagged, 0);
    assert_eq!(same.stored_bytes, 0);
}

#[test]
fn remember_with_tags_adds_to_an_entry_whose_file_changed() {
    let mut entries = FakeEntries::new();
    run(
        FakeKind::new(),
        &Memory::new().file("/t/a.fake", "x|"),
        &mut entries,
        &tagged_request("/t", &["a"], 32),
    )
    .expect("first run");
    entries
        .script
        .insert("file:/t/a.fake".to_owned(), Script::Appended);

    let output = run(
        FakeKind::new(),
        &Memory::new().file("/t/a.fake", "x|y|"),
        &mut entries,
        &tagged_request("/t", &["b"], 32),
    )
    .expect("grown");

    assert_eq!(
        outcomes(&output),
        vec![("/t/a.fake", &Outcome::Appended(id(1)))]
    );
    assert_eq!(entries.seen.len(), 2);
    assert_eq!(entries.adopted.len(), 1);
    assert_eq!(
        entries.metas[&id(1)].labels.tags,
        carried(&[("a", by_person()), ("b", by_person())])
    );
    assert_eq!(output.tagged, 1);
    assert_eq!(
        output.stored_bytes,
        entries.seen[1].bytes.len() as u64 + 100 + ADOPTED
    );
}

#[test]
fn a_file_whose_entry_would_pass_the_cap_is_refused_before_anything_of_it_is_kept() {
    let mut entries = FakeEntries::new();
    run(
        FakeKind::new(),
        &Memory::new().file("/t/a.fake", "x|"),
        &mut entries,
        &tagged_request("/t", &["a"], 1),
    )
    .expect("first run");
    entries
        .script
        .insert("file:/t/a.fake".to_owned(), Script::Appended);

    let grown = Counting::new(Memory::new().file("/t/a.fake", "x|y|"));
    let output = run(
        FakeKind::new(),
        &grown,
        &mut entries,
        &tagged_request("/t", &["b"], 1),
    )
    .expect("grown");

    assert_eq!(
        refusal(&output),
        "its entry would carry more than tag.cap tags (1)"
    );
    assert_eq!(output.tagged, 0);
    assert_eq!(entries.seen.len(), 1);
    assert!(entries.adopted.is_empty());
    assert!(grown.read_paths.borrow().is_empty());
}

#[test]
fn the_files_kept_before_a_refused_one_stay_kept_and_tagged() {
    let mut entries = FakeEntries::new();
    let files = Memory::new()
        .file("/t/a.fake", "x|")
        .file("/t/b.fake", "x|")
        .file("/t/c.fake", "x|");
    run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t/b.fake", &["a"], 1),
    )
    .expect("b alone");

    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t", &["z"], 1),
    )
    .expect("the folder");

    assert_eq!(
        outcomes(&output),
        vec![
            ("/t/a.fake", &Outcome::New(id(2))),
            (
                "/t/b.fake",
                &Outcome::Refused("its entry would carry more than tag.cap tags (1)".to_owned())
            ),
            ("/t/c.fake", &Outcome::New(id(3))),
        ]
    );
    assert_eq!(output.tagged, 2);
    assert_eq!(
        entries.metas[&id(1)].labels.tags,
        carried(&[("a", by_person())])
    );
    assert_eq!(
        entries.metas[&id(2)].labels.tags,
        carried(&[("z", by_person())])
    );
    assert_eq!(
        entries.metas[&id(3)].labels.tags,
        carried(&[("z", by_person())])
    );
}

#[test]
fn remember_makes_a_models_tag_a_persons_and_counts_the_entry_tagged() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = FakeEntries::new();
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("first run");
    let model = Some(Standing::Carried(By::Model));
    entries.metas.get_mut(&id(1)).expect("meta").labels.tags = carried(&[("a", model)]);

    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t", &["a"], 32),
    )
    .expect("a models tag");
    assert_eq!(output.tagged, 1);
    assert_eq!(entries.adopted.len(), 1);
    assert_eq!(
        entries.metas[&id(1)].labels.tags,
        carried(&[("a", by_person())])
    );

    entries.metas.get_mut(&id(1)).expect("meta").labels.tags =
        carried(&[("a", model), ("b", by_person()), ("c", by_person())]);
    let lowered = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t", &["a"], 1),
    )
    .expect("a lowered cap");
    assert_eq!(lowered.tagged, 1);
    assert_eq!(lowered.tally().refused, 0);
    assert_eq!(
        entries.metas[&id(1)].labels.tags,
        carried(&[("a", by_person()), ("b", by_person()), ("c", by_person())])
    );
}

fn held() -> Id {
    Id::parse(HELD).expect("an id")
}

fn with_id(rest: &str) -> String {
    format!("id={HELD}|{rest}")
}

fn old_label() -> CaptureLabel {
    CaptureLabel::new("file:/t/old.fake".into()).expect("a label")
}

fn holder(label: &str) -> FakeEntries {
    FakeEntries::new().kept_under(Some(label), held(), "fake", Tags::default())
}

fn seeded(entries: &mut FakeEntries) -> Head {
    let head = entries
        .heads
        .get_mut("file:/t/old.fake")
        .expect("the holder's head");
    head.generation = Generation(3);
    head.records = 2;
    head.mark = Mark { end: 7, hash: 11 };
    head.stamp = Some(Stamp {
        size: 5,
        modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
    });
    Head {
        stamp: None,
        ..*head
    }
}

fn held_by(label: &str) -> Script {
    Script::IdHeld(CaptureLabel::new(label.into()))
}

fn moves(entries: &FakeEntries) -> Vec<(Id, Option<String>)> {
    entries
        .labelled
        .iter()
        .map(|(id, label)| (*id, label.as_ref().map(ToString::to_string)))
        .collect()
}

#[test]
fn a_file_whose_id_is_held_where_no_file_is_left_moves_its_entry_to_its_label() {
    let files = Memory::new().file("/t/new.fake", with_id("x|").as_bytes());
    let mut entries = FakeEntries::new()
        .kept_under(
            Some("file:/t/old.fake"),
            held(),
            "fake",
            carried(&[("a", by_person())]),
        )
        .on("file:/t/new.fake", Script::Unchanged);
    {
        let meta = entries.metas.get_mut(&held()).expect("the holder");
        meta.labels.id = Some(id(8));
        meta.captured = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    }
    let before = entries.metas[&held()].clone();
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");

    let mut expected = before.clone();
    expected.labels.capture = CaptureLabel::new("file:/t/new.fake".into());
    assert_eq!(entries.metas[&held()], expected);
    assert_eq!(entries.metas[&held()].labels.id, Some(id(8)));
    assert_eq!((entries.seen[0].id, entries.seen[0].id_label), (None, None));

    assert_eq!(
        moves(&entries),
        vec![(held(), Some("file:/t/new.fake".to_string()))]
    );
    let after = &entries.metas[&held()];
    assert_eq!(after.kind, before.kind);
    assert_eq!(after.captured, before.captured);
    assert_eq!(after.labels.id, before.labels.id);
    assert_eq!(after.labels.tags, before.labels.tags);
    assert_eq!(
        after.labels.capture.as_ref().map(ToString::to_string),
        Some("file:/t/new.fake".to_string())
    );
    assert_eq!(entries.seen.len(), 1);
    assert_eq!(entries.seen[0].label, "file:/t/new.fake");
    assert_eq!(entries.seen[0].bytes, with_id("x|").as_bytes());
    assert_eq!(
        outcomes(&output),
        vec![("/t/new.fake", &Outcome::Unchanged(held()))]
    );
    assert_eq!(
        output.items[0].moved,
        Some(Moved {
            id: held(),
            from: old_label(),
            bytes: ADOPTED
        })
    );
    assert_eq!(output.stored_bytes, ADOPTED);
}

#[test]
fn a_file_whose_id_is_held_by_a_file_that_is_still_there_is_refused() {
    let files = Memory::new()
        .file("/t/new.fake", with_id("y|").as_bytes())
        .file("/t/old.fake", with_id("x|").as_bytes());
    let mut entries =
        holder("file:/t/old.fake").on("file:/t/new.fake", held_by("file:/t/old.fake"));
    let (heads, metas) = (entries.heads.clone(), entries.metas.clone());
    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &request("/t/new.fake"),
    )
    .expect("run");

    assert_eq!(
        outcomes(&output),
        vec![(
            "/t/new.fake",
            &Outcome::Refused(format!("its id {HELD} is held by file:/t/old.fake"))
        )]
    );
    assert!(entries.labelled.is_empty());
    assert_eq!((&entries.heads, &entries.metas), (&heads, &metas));
}

#[test]
fn an_old_label_that_holds_a_folder_or_a_link_is_no_file() {
    for old in [
        Memory::new().folder("/t/old.fake"),
        Memory::new().other("/t/old.fake"),
    ] {
        let files = old.file("/t/new.fake", with_id("x|").as_bytes());
        let mut entries = holder("file:/t/old.fake");
        let output = run(
            FakeKind::new(),
            &files,
            &mut entries,
            &request("/t/new.fake"),
        )
        .expect("run");
        assert_eq!(moves(&entries).len(), 1);
        assert!(output.items[0].moved.is_some());
        assert_eq!(output.tally().refused, 0);
    }
}

#[test]
fn an_old_label_that_cannot_be_looked_at_refuses() {
    let files = Memory::new()
        .denied("/t/old.fake")
        .file("/t/new.fake", with_id("x|").as_bytes());
    let mut entries =
        holder("file:/t/old.fake").on("file:/t/new.fake", held_by("file:/t/old.fake"));
    let (heads, metas) = (entries.heads.clone(), entries.metas.clone());
    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &request("/t/new.fake"),
    )
    .expect("run");
    assert_eq!(output.tally().refused, 1);
    assert_eq!(
        outcomes(&output),
        vec![(
            "/t/new.fake",
            &Outcome::Refused(format!("its id {HELD} is held by file:/t/old.fake"))
        )]
    );
    assert!(entries.labelled.is_empty());
    assert_eq!((&entries.heads, &entries.metas), (&heads, &metas));
    assert!(output.items[0].moved.is_none());
}

#[test]
fn an_id_held_with_no_file_label_or_by_another_kind_is_refused() {
    let files = Memory::new().file("/t/new.fake", with_id("x|").as_bytes());
    let rows = [
        (
            FakeEntries::new()
                .kept_under(None, held(), "fake", Tags::default())
                .on("file:/t/new.fake", Script::IdHeld(None)),
            format!("its id {HELD} is held by another entry"),
        ),
        (
            FakeEntries::new()
                .kept_under(Some("tag:not-a-file"), held(), "fake", Tags::default())
                .on("file:/t/new.fake", held_by("tag:not-a-file")),
            format!("its id {HELD} is held by tag:not-a-file"),
        ),
        (
            FakeEntries::new()
                .kept_under(Some("file:/t/old.fake"), held(), "other", Tags::default())
                .on("file:/t/new.fake", held_by("file:/t/old.fake")),
            format!("its id {HELD} is held by file:/t/old.fake"),
        ),
    ];
    for (mut entries, line) in rows {
        let (heads, metas) = (entries.heads.clone(), entries.metas.clone());
        let output = run(
            FakeKind::new(),
            &files,
            &mut entries,
            &request("/t/new.fake"),
        )
        .expect("run");
        assert_eq!(
            outcomes(&output),
            vec![("/t/new.fake", &Outcome::Refused(line))]
        );
        assert!(entries.labelled.is_empty());
        assert_eq!((&entries.heads, &entries.metas), (&heads, &metas));
    }
}

#[test]
fn a_moved_file_that_changed_counts_as_what_its_bytes_give() {
    for (script, want) in [
        (Script::Appended, "appended"),
        (Script::NewGeneration, "new generation"),
    ] {
        let files = Memory::new().file("/t/new.fake", with_id("x|y|").as_bytes());
        let mut entries = holder("file:/t/old.fake").on("file:/t/new.fake", script);
        let head = seeded(&mut entries);
        let output = run(
            FakeKind::new(),
            &files,
            &mut entries,
            &request("/t/new.fake"),
        )
        .expect("run");
        assert_eq!(entries.seen.len(), 1, "{want}");
        assert_eq!(entries.seen[0].label, "file:/t/new.fake", "{want}");
        assert_eq!(entries.seen[0].id, None, "{want}");
        assert_eq!(entries.seen[0].head, Some(head), "{want}");
        assert!(!entries.heads.contains_key("file:/t/old.fake"), "{want}");
        let tally = output.tally();
        assert_eq!(tally.new, 0, "{want}");
        assert_eq!(tally.appended + tally.new_generation, 1, "{want}");
        assert_eq!(tally.appended, u64::from(want == "appended"), "{want}");
        assert!(output.items[0].moved.is_some());
        assert_eq!(
            output.stored_bytes,
            ADOPTED + with_id("x|y|").len() as u64 + 100,
            "{want}"
        );
    }
}

#[test]
fn a_file_moved_twice_and_back_is_one_entry_each_time() {
    let mut entries = holder("file:/t/a.fake")
        .on("file:/t/b.fake", Script::Unchanged)
        .on("file:/t/c.fake", Script::Unchanged)
        .on("file:/t/a.fake", Script::Unchanged);
    for name in ["b", "c", "a"] {
        let files = Memory::new().file(format!("/t/{name}.fake"), with_id("x|").as_bytes());
        run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("run");
    }
    assert_eq!(
        moves(&entries),
        vec![
            (held(), Some("file:/t/b.fake".to_string())),
            (held(), Some("file:/t/c.fake".to_string())),
            (held(), Some("file:/t/a.fake".to_string())),
        ]
    );
    assert_eq!(entries.metas.len(), 1);
}

#[test]
fn a_tagged_run_declares_for_a_moved_entry_and_refuses_a_full_one_after_its_move() {
    let files = Memory::new().file("/t/new.fake", with_id("x|").as_bytes());

    let mut roomy = holder("file:/t/old.fake").on("file:/t/new.fake", Script::Unchanged);
    let output = run(
        FakeKind::new(),
        &files,
        &mut roomy,
        &tagged_request("/t/new.fake", &["x"], 2),
    )
    .expect("run");
    assert_eq!(output.tagged, 1);
    assert!(output.items[0].moved.is_some());
    assert!(
        roomy.metas[&held()]
            .labels
            .tags
            .carried()
            .any(|(tag, _)| tag.as_str() == "x")
    );

    let mut full = FakeEntries::new()
        .kept_under(
            Some("file:/t/old.fake"),
            held(),
            "fake",
            carried(&[("a", by_person()), ("b", by_person())]),
        )
        .on("file:/t/new.fake", Script::Unchanged);
    let output = run(
        FakeKind::new(),
        &files,
        &mut full,
        &tagged_request("/t/new.fake", &["x"], 2),
    )
    .expect("run");
    assert_eq!(
        outcomes(&output),
        vec![(
            "/t/new.fake",
            &Outcome::Refused("its entry would carry more than tag.cap tags (2)".to_string())
        )]
    );
    assert_eq!(
        output.items[0].moved.as_ref().map(|moved| moved.bytes),
        Some(ADOPTED)
    );
    assert_eq!(output.stored_bytes, ADOPTED);
    assert_eq!(moves(&full).len(), 1);
}

#[test]
fn a_file_at_the_old_label_that_carries_another_id_or_none_does_not_hold_the_entry() {
    let other = format!("id={}|z|", id(9));
    let rows: [(Memory, bool); 4] = [
        (Memory::new().file("/t/old.fake", other.as_bytes()), true),
        (Memory::new().file("/t/old.fake", "z|"), true),
        (Memory::new().file("/t/old.fake", "!refused"), false),
        (Memory::new().unreadable("/t/old.fake", Node::File), false),
    ];
    for (old, moves_it) in rows {
        let files = old.file("/t/new.fake", with_id("x|").as_bytes());
        let mut entries = holder("file:/t/old.fake");
        let output = run(
            FakeKind::new(),
            &files,
            &mut entries,
            &request("/t/new.fake"),
        )
        .expect("run");
        assert_eq!(output.items[0].moved.is_some(), moves_it);
        assert_eq!(moves(&entries).len(), usize::from(moves_it));
        assert_eq!(output.tally().refused, u64::from(!moves_it));
    }
}

#[test]
fn a_file_whose_label_holds_one_entry_and_whose_id_another_holds_is_refused_and_nothing_is_written()
{
    let files = Memory::new().file("/t/s.fake", with_id("x|").as_bytes());
    let mine = id(7);
    for (label, line) in [
        (
            Some("file:/t/g.fake"),
            format!(
                "its id {HELD} is held by file:/t/g.fake, but this path was kept as entry {mine}"
            ),
        ),
        (
            None,
            format!(
                "its id {HELD} is held by another entry, but this path was kept as entry {mine}"
            ),
        ),
    ] {
        let mut entries = FakeEntries::new()
            .kept_under(Some("file:/t/s.fake"), mine, "fake", Tags::default())
            .kept_under(label, held(), "fake", Tags::default());
        let output =
            run(FakeKind::new(), &files, &mut entries, &request("/t/s.fake")).expect("run");
        assert_eq!(
            outcomes(&output),
            vec![("/t/s.fake", &Outcome::Refused(line))]
        );
        assert_eq!(output.items[0].carried, Some(held()));
        assert!(entries.labelled.is_empty());
        assert!(entries.seen.is_empty());
        assert_eq!(entries.heads["file:/t/s.fake"].id, mine);
    }

    let mut entries = FakeEntries::new()
        .kept_under(Some("file:/t/s.fake"), mine, "fake", Tags::default())
        .on("file:/t/s.fake", Script::Appended);
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t/s.fake")).expect("run");
    assert_eq!(
        outcomes(&output),
        vec![("/t/s.fake", &Outcome::Appended(mine))]
    );
    assert_eq!(output.tally().appended, 1);
    assert_eq!(output.items[0].carried, None);
    assert_eq!(entries.seen.len(), 1);
    assert_eq!(entries.seen[0].id, None);
    assert!(entries.labelled.is_empty());
}

#[test]
fn a_moved_file_is_skipped_unread_by_a_later_run() {
    let files = Memory::new().file("/t/new.fake", with_id("x|").as_bytes());
    let mut entries = holder("file:/t/old.fake").on("file:/t/new.fake", Script::Unchanged);
    run(FakeKind::new(), &files, &mut entries, &request("/t")).expect("the move");

    let again = Counting::new(files);
    let output = run(FakeKind::new(), &again, &mut entries, &request("/t")).expect("again");
    assert_eq!(again.read_paths.borrow().len(), 0);
    assert_eq!(
        outcomes(&output),
        vec![("/t/new.fake", &Outcome::Unchanged(held()))]
    );
    assert_eq!(moves(&entries).len(), 1);
    assert_eq!(entries.seen.len(), 1);
}

#[test]
fn a_rename_that_changes_only_the_case_of_a_name_moves_the_entry() {
    let files = Counting::new(
        Memory::new()
            .file("/t/N.fake", with_id("x|").as_bytes())
            .alias("/t/n.fake", "/t/N.fake"),
    );
    let mut entries = holder("file:/t/n.fake");
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t/N.fake")).expect("run");
    assert_eq!(
        moves(&entries),
        vec![(held(), Some("file:/t/N.fake".to_string()))]
    );
    assert_eq!(output.tally().refused, 0);
    assert_eq!(Counting::count(&files.read_paths, "/t/n.fake"), 0);
}

#[test]
fn a_run_stopped_between_a_moves_two_writes_is_finished_by_the_next_run() {
    let files = Memory::new().file("/t/new.fake", with_id("x|").as_bytes());
    let mut entries = holder("file:/t/old.fake").on("file:/t/new.fake", Script::Failure);
    let head = seeded(&mut entries);
    let error = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &request("/t/new.fake"),
    )
    .expect_err("the capture fails");
    assert!(matches!(error, Error::Store(_)));
    assert_eq!(error.to_string(), "stopped at the test hook Appended");
    assert_eq!(entries.seen.len(), 1);
    assert_eq!(entries.seen[0].head, Some(head));
    assert_eq!(moves(&entries).len(), 1);
    assert!(!entries.heads.contains_key("file:/t/old.fake"));
    assert_eq!(entries.heads["file:/t/new.fake"], head);
    assert_eq!(
        entries.metas[&held()].labels.capture,
        CaptureLabel::new("file:/t/new.fake".into())
    );

    entries
        .script
        .insert("file:/t/new.fake".to_string(), Script::Unchanged);
    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &request("/t/new.fake"),
    )
    .expect("the next run");
    assert_eq!(moves(&entries).len(), 1, "no second move");
    assert_eq!(
        outcomes(&output),
        vec![("/t/new.fake", &Outcome::Unchanged(held()))]
    );
    assert!(output.items[0].moved.is_none());
    assert_eq!(entries.seen.len(), 2);
    assert_eq!(entries.seen[1].label, "file:/t/new.fake");
    assert_eq!(entries.seen[1].id, None);
    assert_eq!(entries.seen[1].head, Some(head));
    assert_eq!(entries.metas.len(), 1);
}

#[test]
fn tags_a_moved_entry_already_carries_are_taken_at_the_cap() {
    let files = Memory::new().file("/t/new.fake", with_id("x|").as_bytes());
    let mut entries = FakeEntries::new()
        .kept_under(
            Some("file:/t/old.fake"),
            held(),
            "fake",
            carried(&[("a", by_person()), ("b", by_person())]),
        )
        .on("file:/t/new.fake", Script::Unchanged);
    let output = run(
        FakeKind::new(),
        &files,
        &mut entries,
        &tagged_request("/t/new.fake", &["a", "b"], 1),
    )
    .expect("run");
    assert_eq!(
        outcomes(&output),
        vec![("/t/new.fake", &Outcome::Unchanged(held()))]
    );
    assert_eq!(output.tally().refused, 0);
    assert!(output.items[0].moved.is_some());
    assert_eq!(
        moves(&entries),
        vec![(held(), Some("file:/t/new.fake".to_string()))]
    );
    assert_eq!(entries.seen.len(), 1);
    assert_eq!(entries.seen[0].label, "file:/t/new.fake");
    assert_eq!(
        entries.metas[&held()].labels.tags,
        carried(&[("a", by_person()), ("b", by_person())])
    );
}

#[test]
fn a_folder_renamed_only_in_case_moves_its_entries() {
    let files = Memory::new()
        .file("/t/shed/n.fake", with_id("x|").as_bytes())
        .alias("/t/Shed/n.fake", "/t/shed/n.fake")
        .alias("/t/Shed", "/t/shed");
    assert_eq!(
        files.list(Path::new("/t/Shed")).expect("a list"),
        [std::ffi::OsString::from("n.fake")]
    );
    assert_eq!(
        files.node(Path::new("/t/Shed/n.fake")).ok(),
        Some(Node::File)
    );
    assert_eq!(
        files.canonical(Path::new("/t/Shed/n.fake")).ok(),
        Some(PathBuf::from("/t/shed/n.fake"))
    );
    let mut entries = holder("file:/t/Shed/n.fake");
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t/shed")).expect("run");
    assert_eq!(
        moves(&entries),
        vec![(held(), Some("file:/t/shed/n.fake".to_string()))]
    );
    assert_eq!(output.tally().refused, 0);
}

#[test]
fn a_third_file_with_the_id_is_refused_while_the_renamed_one_answers_at_the_old_spelling() {
    let files = Memory::new()
        .file("/t/N.fake", with_id("x|").as_bytes())
        .alias("/t/n.fake", "/t/N.fake")
        .file("/t/z.fake", with_id("z|").as_bytes());
    let mut entries = holder("file:/t/n.fake");
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t/z.fake")).expect("z");
    assert_eq!(
        outcomes(&output),
        vec![(
            "/t/z.fake",
            &Outcome::Refused(format!("its id {HELD} is held by file:/t/n.fake"))
        )]
    );
    assert!(entries.labelled.is_empty());

    let output = run(FakeKind::new(), &files, &mut entries, &request("/t/N.fake")).expect("N");
    assert_eq!(moves(&entries).len(), 1);
    assert!(output.items[0].moved.is_some());
}

#[test]
fn a_cap_refusal_held_back_is_kept_when_the_file_is_gone_at_its_read() {
    let cap = "its entry would carry more than tag.cap tags (1)".to_string();
    let rows = [
        (
            "/t/new.fake",
            Memory::new().file("/t/new.fake", "x|"),
            Memory::new(),
        ),
        (
            "/t/a.fake.z",
            Memory::new().file("/t/a.fake.z", "zx|"),
            Memory::new(),
        ),
        (
            "/t/a.fake",
            Memory::new().file("/t/a.fake", "x|"),
            Memory::new().denied("/t/a.fake.z"),
        ),
    ];
    for (file, before, after) in rows {
        let files = Turning::new(before, after, Call::Stamp, file);
        let mut entries = FakeEntries::new();
        let output = run(
            FakeKind::new(),
            &files,
            &mut entries,
            &tagged_request(file, &["a", "b"], 1),
        )
        .expect("run");
        assert_eq!(
            outcomes(&output),
            vec![(file, &Outcome::Refused(cap.clone()))],
            "{file}"
        );
        assert!(output.items[0].moved.is_none(), "{file}");
        assert!(entries.seen.is_empty(), "{file}");
        assert!(entries.labelled.is_empty(), "{file}");
    }
}

fn pass(files: &dyn Files, entries: &mut FakeEntries, paths: &[&str]) -> Vec<Output> {
    pass_of(FakeKind::promising(), files, entries, paths)
}

fn pass_of(
    kind: FakeKind,
    files: &dyn Files,
    entries: &mut FakeEntries,
    paths: &[&str],
) -> Vec<Output> {
    let registry = kinds(kind);
    let paths: Vec<Follow> = paths
        .iter()
        .map(|path| Follow {
            path: PathBuf::from(path),
            kind: None,
        })
        .collect();
    let planned = follows::plan(&paths, &registry, files);
    follows::keep(&planned, files, entries).expect("a pass")
}

fn emptied(entries: &FakeEntries) -> Vec<&str> {
    entries
        .seen
        .iter()
        .filter(|seen| seen.ends.is_empty())
        .map(|seen| seen.label.as_str())
        .collect()
}

fn entry(entries: FakeEntries, label: &str, n: u8) -> FakeEntries {
    entries.kept_under(Some(label), id(n), "fake", Tags::default())
}

#[test]
fn a_follow_withdraws_the_entry_of_a_gone_file_when_its_kind_says_so() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = entry(
        entry(FakeEntries::new().resolving(), "file:/t/a.fake", 5),
        "file:/t/gone.fake",
        6,
    );
    entries
        .heads
        .get_mut("file:/t/gone.fake")
        .expect("a head")
        .generation = Generation(4);
    let outputs = pass(&files, &mut entries, &["/t"]);

    assert_eq!(emptied(&entries), ["file:/t/gone.fake"]);
    let after = entries.heads["file:/t/gone.fake"];
    assert_eq!((after.generation, after.records), (Generation(5), 0));
    let seen = entries
        .seen
        .iter()
        .find(|seen| seen.ends.is_empty())
        .expect("one");
    assert_eq!((seen.stamp, seen.bytes.len(), seen.id), (None, 0, None));
    assert!(entries.labelled.is_empty() && entries.adopted.is_empty());
    let item = outputs[0].items.last().expect("an item");
    assert_eq!(item.path, Path::new("/t/gone.fake"));
    assert_eq!(item.outcome, Outcome::NewGeneration(id(6)));
    assert_eq!(item.withdrew, Some(Generation(4)));
}

#[test]
fn a_kind_that_does_not_say_so_keeps_every_entry_and_the_store_is_not_asked() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = entry(FakeEntries::new(), "file:/t/gone.fake", 6);
    pass_of(FakeKind::new(), &files, &mut entries, &["/t"]);
    assert_eq!(entries.seen.len(), 1);
    assert!(emptied(&entries).is_empty());
}

#[test]
fn a_follow_that_claims_no_file_withdraws_nothing() {
    let files = Memory::new().folder("/t");
    let mut entries = entry(
        entry(FakeEntries::new().resolving(), "file:/t/a.fake", 1),
        "file:/t/b.fake",
        2,
    );
    let registry = kinds(FakeKind::promising());
    let named = [Follow {
        path: PathBuf::from("/t"),
        kind: Some("fake".to_string()),
    }];
    let planned = follows::plan(&named, &registry, &files);
    let outputs = follows::keep(&planned, &files, &mut entries).expect("a pass");
    assert!(entries.seen.is_empty());
    assert!(outputs[0].items.is_empty());
}

#[test]
fn a_missing_follow_and_a_follow_of_one_file_withdraw_nothing() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = entry(
        entry(FakeEntries::new().resolving(), "file:/gone/b.fake", 1),
        "file:/t/gone.fake",
        2,
    );
    pass(&files, &mut entries, &["/gone", "/t/a.fake"]);
    assert!(emptied(&entries).is_empty());
    assert_eq!(entries.seen.len(), 1);
    assert_eq!(entries.seen[0].label, "file:/t/a.fake");

    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = entry(
        entry(FakeEntries::new().resolving(), "file:/t/a.fake/one.fake", 8),
        "file:/t/a.fake/two.fake",
        9,
    );
    let heads = entries.heads.clone();
    pass(&files, &mut entries, &["/t/a.fake"]);
    assert!(emptied(&entries).is_empty());
    assert_eq!(
        entries.heads["file:/t/a.fake/one.fake"],
        heads["file:/t/a.fake/one.fake"]
    );
    assert_eq!(
        entries.heads["file:/t/a.fake/two.fake"],
        heads["file:/t/a.fake/two.fake"]
    );
    assert_eq!(entries.seen.len(), 1);
    assert_eq!(entries.seen[0].label, "file:/t/a.fake");
}

#[test]
fn only_a_path_with_nothing_at_it_is_gone() {
    let files = Memory::new()
        .file("/t/keep.fake", "x|")
        .denied("/t/denied.fake")
        .folder("/t/dir.fake")
        .other("/t/link.fake");
    let mut entries = FakeEntries::new().resolving();
    for (n, label) in [
        "file:/t/denied.fake",
        "file:/t/dir.fake",
        "file:/t/link.fake",
        "file:/t/gone.fake",
    ]
    .into_iter()
    .enumerate()
    {
        entries = entry(entries, label, n as u8 + 1);
    }
    pass(&files, &mut entries, &["/t"]);
    assert_eq!(emptied(&entries), ["file:/t/gone.fake"]);
}

#[test]
fn an_entry_that_is_already_empty_is_left_alone() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = entry(FakeEntries::new().resolving(), "file:/t/gone.fake", 6);
    entries
        .heads
        .get_mut("file:/t/gone.fake")
        .expect("a head")
        .records = 0;
    let outputs = pass(&files, &mut entries, &["/t"]);
    assert!(emptied(&entries).is_empty());
    assert_eq!(outputs[0].items.len(), 1);
}

#[test]
fn an_entry_of_another_kind_or_from_another_folder_is_left_alone() {
    let files = Memory::new().file("/t/a.fake", "x|");
    let mut entries = FakeEntries::new().resolving().kept_under(
        Some("file:/t/other.fake"),
        id(1),
        "other",
        Tags::default(),
    );
    for (n, label) in ["file:/t2/g.fake", "file:/T/g.fake", "file:/x/t/g.fake"]
        .into_iter()
        .enumerate()
    {
        entries = entry(entries, label, n as u8 + 2);
    }
    pass(&files, &mut entries, &["/t"]);
    assert!(emptied(&entries).is_empty());
}

#[test]
fn an_entry_claimed_under_another_of_its_files_is_not_gone() {
    let files = Memory::new()
        .file("/t/a.fake.z", "zx|")
        .file("/t/keep.fake", "x|");
    let mut entries = entry(FakeEntries::new().resolving(), "file:/t/a.fake", 3)
        .on("file:/t/a.fake", Script::Appended);
    let outputs = pass(&files, &mut entries, &["/t"]);
    assert!(emptied(&entries).is_empty());
    assert!(
        entries
            .seen
            .iter()
            .any(|seen| seen.label == "file:/t/a.fake" && seen.bytes == b"x|")
    );
    assert_eq!(outputs[0].items[0].path, Path::new("/t/a.fake.z"));
    assert_eq!(outputs[0].items[0].outcome, Outcome::Appended(id(3)));
    assert_eq!(outputs[0].tally().refused, 0);
}

#[test]
fn an_entry_that_moved_in_this_pass_is_not_withdrawn() {
    let files = Memory::new().file("/t/new.fake", with_id("x|").as_bytes());
    let mut entries = holder("file:/t/old.fake").resolving();
    pass(&files, &mut entries, &["/t"]);
    assert_eq!(moves(&entries).len(), 1);
    assert!(emptied(&entries).is_empty());
}

#[test]
fn withdrawn_entries_are_items_after_the_claimed_files_in_path_order() {
    let files = Memory::new().file("/t/x.fake", "x|");
    let mut entries = entry(
        entry(FakeEntries::new().resolving(), "file:/t/m.fake", 9),
        "file:/t/z.fake",
        2,
    );
    let outputs = pass(&files, &mut entries, &["/t"]);
    let output = &outputs[0];
    let paths: Vec<&str> = output
        .items
        .iter()
        .map(|item| item.path.to_str().expect("utf-8"))
        .collect();
    assert_eq!(paths, ["/t/x.fake", "/t/m.fake", "/t/z.fake"]);
    assert_eq!(output.tally().new_generation, 2);
    assert_eq!((output.records, output.raw_bytes), (1, 2));
    assert_eq!(output.stored_bytes, (2 + 100) + 100 + 100);
}

#[test]
fn an_entry_moved_from_one_follow_to_another_is_moved_and_not_withdrawn_in_either_order() {
    for order in [["/a", "/b"], ["/b", "/a"]] {
        let files = Memory::new()
            .file("/a/keep.fake", "x|")
            .file("/b/new.fake", with_id("x|").as_bytes());
        let mut entries = holder("file:/a/old.fake").resolving();
        let outputs = pass(&files, &mut entries, &order);
        assert_eq!(moves(&entries).len(), 1, "{order:?}");
        assert!(emptied(&entries).is_empty(), "{order:?}");
        let with_move: Vec<&Output> = outputs
            .iter()
            .filter(|output| output.items.iter().any(|item| item.moved.is_some()))
            .collect();
        assert_eq!(with_move.len(), 1, "{order:?}");
        assert_eq!(with_move[0].target, Path::new("/b"), "{order:?}");
    }
}

#[test]
fn a_follow_whose_claimed_files_were_all_refused_withdraws_nothing() {
    let before = Memory::new().file("/t/a.fake", "x|");
    let files = Turning::new(before, Memory::new().folder("/t"), Call::Stamp, "/t/a.fake");
    let mut entries = entry(
        entry(FakeEntries::new().resolving(), "file:/t/b.fake", 1),
        "file:/t/c.fake",
        2,
    );
    let outputs = pass(&files, &mut entries, &["/t"]);
    assert_eq!(outputs[0].tally().refused, 1);
    assert!(emptied(&entries).is_empty());
}

#[test]
fn a_follow_above_another_withdraws_under_it_when_it_still_shows_a_file() {
    for sub_shows_a_file in [true, false] {
        let mut files = Memory::new().file("/n/x.fake", "x|").folder("/n/sub");
        if sub_shows_a_file {
            files = files.file("/n/sub/y.fake", "x|");
        }
        let mut entries = entry(FakeEntries::new().resolving(), "file:/n/sub/gone.fake", 4);
        let outputs = pass(&files, &mut entries, &["/n", "/n/sub"]);
        assert_eq!(
            emptied(&entries),
            ["file:/n/sub/gone.fake"],
            "{sub_shows_a_file}"
        );
        let said: Vec<(&Path, &Path)> = outputs
            .iter()
            .flat_map(|output| {
                output
                    .items
                    .iter()
                    .filter(|item| item.withdrew.is_some())
                    .map(move |item| (output.target.as_path(), item.path.as_path()))
            })
            .collect();
        assert_eq!(
            said,
            [(Path::new("/n"), Path::new("/n/sub/gone.fake"))],
            "{sub_shows_a_file}"
        );
    }
}

#[test]
fn an_entry_whose_name_the_volume_folds_onto_another_file_is_gone() {
    let files = Memory::new()
        .file("/t/N.fake", "x|")
        .alias("/t/n.fake", "/t/N.fake");
    let mut entries = entry(FakeEntries::new().resolving(), "file:/t/n.fake", 3);
    pass(&files, &mut entries, &["/t"]);
    assert_eq!(emptied(&entries), ["file:/t/n.fake"]);
}

#[test]
fn a_follow_of_the_root_withdraws_under_it() {
    let files = Memory::new().file("/a.fake", "x|");
    let mut entries = entry(FakeEntries::new().resolving(), "file:/d/b.fake", 3);
    pass(&files, &mut entries, &["/"]);
    assert_eq!(emptied(&entries), ["file:/d/b.fake"]);
}

#[test]
fn an_entry_whose_file_was_refused_under_another_name_is_not_withdrawn_in_that_pass() {
    let files = Memory::new()
        .file("/t/b.fake", with_id("x|").as_bytes())
        .file("/t/c.fake", "x|");
    let (a, b) = (held(), id(7));
    let mut entries = FakeEntries::new()
        .resolving()
        .kept_under(Some("file:/t/a.fake"), a, "fake", Tags::default())
        .kept_under(Some("file:/t/b.fake"), b, "fake", Tags::default());
    entries
        .heads
        .get_mut("file:/t/b.fake")
        .expect("a head")
        .records = 0;
    let (heads, metas) = (entries.heads.clone(), entries.metas.clone());
    let outputs = pass(&files, &mut entries, &["/t"]);
    let refused = outputs[0]
        .items
        .iter()
        .find(|item| item.path == Path::new("/t/b.fake"))
        .expect("an item");
    assert_eq!(
        refused.outcome,
        Outcome::Refused(format!(
            "its id {HELD} is held by file:/t/a.fake, but this path was kept as entry {b}"
        ))
    );
    assert_eq!(refused.carried, Some(a));
    assert_eq!(entries.metas[&b], metas[&b]);
    assert!(emptied(&entries).is_empty());
    assert_eq!(entries.heads["file:/t/a.fake"], heads["file:/t/a.fake"]);
    assert_eq!(entries.heads["file:/t/b.fake"], heads["file:/t/b.fake"]);
    assert_eq!(entries.metas[&a], metas[&a]);

    for order in [["/a", "/b"], ["/b", "/a"]] {
        let files = Memory::new()
            .file("/b/b.fake", with_id("x|").as_bytes())
            .file("/a/s.fake", "x|");
        let mut entries = FakeEntries::new()
            .resolving()
            .kept_under(Some("file:/a/a.fake"), a, "fake", Tags::default())
            .kept_under(Some("file:/b/b.fake"), b, "fake", Tags::default());
        entries
            .heads
            .get_mut("file:/b/b.fake")
            .expect("a head")
            .records = 0;
        let (heads, metas) = (entries.heads.clone(), entries.metas.clone());
        let outputs = pass(&files, &mut entries, &order);
        assert!(emptied(&entries).is_empty(), "{order:?}");
        let refused = outputs
            .iter()
            .find(|output| output.target == Path::new("/b"))
            .and_then(|output| {
                output
                    .items
                    .iter()
                    .find(|item| item.path == Path::new("/b/b.fake"))
            })
            .expect("an item of /b");
        assert_eq!(
            refused.outcome,
            Outcome::Refused(format!(
                "its id {HELD} is held by file:/a/a.fake, but this path was kept as entry {b}"
            )),
            "{order:?}"
        );
        assert_eq!(refused.carried, Some(a), "{order:?}");
        for label in ["file:/a/a.fake", "file:/b/b.fake"] {
            assert_eq!(entries.heads[label], heads[label], "{order:?}");
        }
        for entry in [a, b] {
            assert_eq!(entries.metas[&entry], metas[&entry], "{order:?}");
        }
    }
}

#[test]
fn an_entry_whose_file_took_a_name_its_kind_does_not_claim_is_gone() {
    let files = Memory::new()
        .file("/t/n.FAKE", "x|")
        .alias("/t/n.fake", "/t/n.FAKE")
        .file("/t/keep.fake", "x|");
    let mut entries = entry(FakeEntries::new().resolving(), "file:/t/n.fake", 3);
    pass(&files, &mut entries, &["/t"]);
    assert_eq!(emptied(&entries), ["file:/t/n.fake"]);
    assert!(
        entries
            .seen
            .iter()
            .all(|seen| seen.label != "file:/t/n.FAKE")
    );
}

#[test]
fn a_withdrawal_the_store_does_not_make_is_a_refusal_of_the_gone_path() {
    let rows = [
        (
            Script::Records("bad ends".to_string()),
            "its records do not cover it: bad ends".to_string(),
        ),
        (
            Script::IdHeld(None),
            format!("its id {} is held by another entry", id(6)),
        ),
        (Script::Unchanged, NOT_WITHDRAWN.to_string()),
        (Script::Appended, NOT_WITHDRAWN.to_string()),
        (Script::New, NOT_WITHDRAWN.to_string()),
    ];
    for (n, (script, reason)) in rows.into_iter().enumerate() {
        let files = Memory::new().file("/t/a.fake", "x|");
        let mut entries = entry(
            entry(FakeEntries::new().resolving(), "file:/t/a.fake", 5),
            "file:/t/gone.fake",
            6,
        )
        .on("file:/t/gone.fake", script);
        let head = entries.heads["file:/t/gone.fake"];
        let outputs = pass(&files, &mut entries, &["/t"]);
        let output = &outputs[0];
        let item = output.items.last().expect("an item");
        assert_eq!(item.path, Path::new("/t/gone.fake"), "row {}", n + 1);
        assert_eq!(item.outcome, Outcome::Refused(reason), "row {}", n + 1);
        assert_eq!(
            (item.withdrew, item.moved.is_none(), item.carried),
            (None, true, None),
            "row {}",
            n + 1
        );
        assert_eq!(
            (output.tally().refused, output.tally().new_generation),
            (1, 0),
            "row {}",
            n + 1
        );
        assert_eq!(
            (output.records, output.raw_bytes, output.stored_bytes),
            (1, 2, 102),
            "row {}",
            n + 1
        );
        if n < 2 {
            assert_eq!(entries.heads["file:/t/gone.fake"], head, "row {}", n + 1);
        }
    }
}

const NOT_WITHDRAWN: &str = "its file is gone, and the store did not withdraw its entry";

fn document_label() -> CaptureLabel {
    CaptureLabel::new(format!("doc:{}", id(5))).expect("a label")
}

fn document_record(text: &str) -> Vec<u8> {
    format!("{{\"text\":\"{text}\"}}\n").into_bytes()
}

fn keep_document(entries: &mut FakeEntries, record: &[u8], tags: &Tagging) -> WrittenAnswer {
    let label = document_label();
    remember::keep_written(
        entries,
        &Written {
            kind: "fake",
            label: &label,
            id: id(5),
            record,
            tags,
        },
    )
}

fn garden(cap: usize) -> Tagging {
    Tagging {
        tags: [Tag::new("garden").expect("a tag")].into(),
        cap,
    }
}

#[test]
fn a_written_text_is_one_new_entry_under_the_label_and_the_id_it_is_given() {
    let mut entries = FakeEntries::new();
    let record = document_record("The beet seeds are in the drawer.");
    let answer = keep_document(&mut entries, &record, &garden(3));

    assert!(matches!(
        answer,
        WrittenAnswer::Kept {
            tagged: true,
            records: 1,
            ..
        }
    ));
    let WrittenAnswer::Kept {
        id: kept,
        raw_bytes,
        stored_bytes,
        ..
    } = answer
    else {
        panic!("{answer:?}");
    };
    assert_eq!(kept, id(5));
    assert_eq!(raw_bytes, record.len() as u64);
    assert_eq!(stored_bytes, record.len() as u64 + 100);
    assert_eq!(entries.seen.len(), 1);
    let seen = &entries.seen[0];
    assert_eq!(seen.label, document_label().as_str());
    assert_eq!(seen.kind, "fake");
    assert_eq!(
        (seen.id, seen.id_label, seen.stamp),
        (Some(id(5)), None, None)
    );
    assert_eq!(seen.bytes, record);
    assert_eq!(seen.ends, [record.len()]);
    assert_eq!(
        entries.metas[&id(5)].labels.tags,
        carried(&[("garden", by_person())])
    );
    assert_eq!(*entries.looked.borrow(), [document_label().to_string()]);
    assert_eq!(seen.lookups_before, 1);
    assert!(entries.labelled.is_empty() && entries.adopted.is_empty());
}

#[test]
fn a_written_text_with_more_tags_than_the_cap_is_refused_before_the_store_is_asked() {
    let mut entries = FakeEntries::new();
    let tags = Tagging {
        tags: [
            Tag::new("garden").expect("a tag"),
            Tag::new("beets").expect("a tag"),
        ]
        .into(),
        cap: 1,
    };
    let answer = keep_document(&mut entries, &document_record("x"), &tags);
    let WrittenAnswer::Refused(reason) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(reason, "its entry would carry more than tag.cap tags (1)");
    assert!(entries.seen.is_empty());
    assert!(entries.looked.borrow().is_empty());
}

#[test]
fn a_store_that_refuses_or_fails_a_written_text_says_so() {
    let record = document_record("x");
    let label = document_label().to_string();

    let mut refusing = FakeEntries::new().on(&label, Script::Records("bad ends".to_string()));
    let answer = keep_document(&mut refusing, &record, &Tagging::default());
    assert!(
        matches!(&answer, WrittenAnswer::Refused(text) if text == "its records do not cover it: bad ends"),
        "{answer:?}"
    );

    let mut blind = FakeEntries::new();
    blind.lookup_fails = true;
    let answer = keep_document(&mut blind, &record, &Tagging::default());
    assert!(
        matches!(&answer, WrittenAnswer::BeforeCapture(error) if error.to_string() == "stopped at the test hook by_capture"),
        "{answer:?}"
    );
    assert!(blind.seen.is_empty());

    for script in [Script::Unchanged, Script::Appended, Script::NewGeneration] {
        let mut entries = FakeEntries::new().on(&label, script);
        let answer = keep_document(&mut entries, &record, &Tagging::default());
        assert!(
            matches!(&answer, WrittenAnswer::AtCapture { id: tried, error } if *tried == id(5) && error.to_string() == "the store did not make a new entry for the note"),
            "{answer:?}"
        );
        assert_eq!(entries.seen.len(), 1);
    }

    let mut failing = FakeEntries::new().on(&label, Script::Failure);
    let answer = keep_document(&mut failing, &record, &Tagging::default());
    assert!(
        matches!(&answer, WrittenAnswer::AtCapture { id: tried, error } if *tried == id(5) && error.to_string() == "stopped at the test hook Appended"),
        "{answer:?}"
    );
}

#[test]
fn a_written_text_the_store_would_change_is_refused_before_the_store_is_asked() {
    let mut entries = FakeEntries::new();
    let answer = keep_document(
        &mut entries,
        &document_record(&"z".repeat(256)),
        &Tagging::default(),
    );
    let WrittenAnswer::Refused(reason) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        reason,
        "the store would keep only a placeholder for its text, which it takes for encoded data"
    );
    assert!(entries.looked.borrow().is_empty());
    assert!(entries.seen.is_empty());

    for text in ["0123456789abcdef".repeat(16), "word ".repeat(80)] {
        let mut entries = FakeEntries::new();
        let answer = keep_document(&mut entries, &document_record(&text), &Tagging::default());
        assert!(matches!(answer, WrittenAnswer::Kept { .. }), "{answer:?}");
        assert_eq!(entries.seen.len(), 1);
    }
}

#[test]
fn a_written_text_whose_label_holds_an_entry_is_refused() {
    let label = document_label();
    let mut entries =
        FakeEntries::new().kept_under(Some(label.as_str()), id(7), "fake", Tags::default());
    let head = entries.heads[label.as_str()];
    let answer = keep_document(&mut entries, &document_record("x"), &Tagging::default());
    let WrittenAnswer::Refused(reason) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        reason,
        format!("its id {} is taken by another entry", id(5))
    );
    assert_eq!(*entries.looked.borrow(), [label.to_string()]);
    assert!(entries.seen.is_empty());
    assert_eq!(entries.heads[label.as_str()], head);
}
