mod fakes;

use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Memory, Node};
use bilbo_kinds::kind::Kind;
use bilbo_kinds::remember::{self, Error, Item, Outcome, Output, Request, Tally};
use bilbo_store::entries::Origin;
use fakes::{FakeEntries, FakeKind, Script, id};

const HELD: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";

fn kinds(kind: FakeKind) -> Vec<Box<dyn Kind>> {
    vec![Box::new(kind)]
}

fn request(target: &str, project: Option<&str>) -> Request {
    Request {
        target: PathBuf::from(target),
        kind: None,
        project: project.map(str::to_string),
    }
}

fn run(
    kind: FakeKind,
    files: &Memory,
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
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t", None)).expect("run");

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
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t", None)).expect("run");

    assert_eq!(output.records, 2 + 3);
    assert_eq!(output.raw_bytes, 4 + 6);
    assert_eq!(output.stored_bytes, (4 + 100) + (6 + 100));
}

#[test]
fn the_kinds_proposal_and_the_project_reach_the_capture() {
    let body = format!("id={HELD}|rest|");
    let files = Memory::new()
        .file("/t/a.fake", body.as_bytes())
        .file("/t/b.fake", "no id here|");
    let mut entries = FakeEntries::new();
    run(
        FakeKind::new(),
        &files,
        &mut entries,
        &request("/t", Some("bilbo")),
    )
    .expect("run");

    assert_eq!(entries.seen.len(), 2);
    let first = &entries.seen[0];
    assert_eq!(first.origin, "file:/t/a.fake");
    assert_eq!(first.kind, "fake");
    assert_eq!(first.id, bilbo_store::id::Id::parse(HELD));
    assert_eq!(first.labels, vec!["bilbo".to_string()]);
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
            Script::IdHeld(Origin::new("file:/t/other.md".into())),
        )
        .on(
            "file:/t/d-records.fake",
            Script::Records("end 3 is past the bytes".into()),
        );
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t", None)).expect("run");

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
    let output = run(kind, &files, &mut entries, &request("/t", None)).expect("run");

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
    let error = run(FakeKind::new(), &files, &mut entries, &request("/t", None))
        .expect_err("the run stops");

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
    let output = run(FakeKind::new(), &files, &mut entries, &request("/t", None)).expect("run");

    let order: Vec<&str> = output
        .items
        .iter()
        .map(|item| item.path.to_str().expect("utf-8"))
        .collect();
    let seen: Vec<&str> = entries.seen.iter().map(|s| s.origin.as_str()).collect();
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
    let plan = remember::plan(&request("/t", None), &registry, &files).expect("plan");
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
