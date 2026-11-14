mod fakes;
mod read;

use std::path::{Path, PathBuf};

use std::time::{Duration, SystemTime};

use bilbo_kinds::files::{Call, Files, Memory, Node, Turning};
use bilbo_kinds::follows::{self, Follow, Planned};
use bilbo_kinds::kind::Kind;
use bilbo_kinds::remember::{self, Error, Item, Outcome, Output, Request, Tally};
use bilbo_store::entries::Stamp;
use bilbo_store::labels::{CaptureLabel, StoredLabels};
use fakes::{ADOPTED, Counting, FakeEntries, FakeKind, Script, id};

const HELD: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

fn kinds(kind: FakeKind) -> Vec<Box<dyn Kind>> {
    vec![Box::new(kind)]
}

fn request(target: &str) -> Request {
    Request {
        target: PathBuf::from(target),
        kind: None,
    }
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
        .file("/t/c-held.fake", "x|")
        .file("/t/d-records.fake", "x|")
        .file(bad.clone(), "x|")
        .file("/t/z-fine.fake", "x|");
    let mut entries = FakeEntries::new()
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
        }
    );
}

#[test]
fn a_bare_run_keeps_each_follow_in_order_and_skips_a_missing_folder() {
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
