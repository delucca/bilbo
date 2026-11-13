use std::cell::Cell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use bilbo_search::analyze::{self, Omit};
use bilbo_search::bm25::Bm25;
use bilbo_search::chunk::{self, Chunk, Size};
use bilbo_search::index::{Doc, Extent, Index, IndexError};
use bilbo_search::lookup::{Filter, Lookup};
use bilbo_search::position::{Manifest, Position};
use bilbo_search::rank::{Candidate, Query, Ranker};
use bilbo_store::entries::{Cursor, Generation, RecordRange};
use bilbo_store::id::Id;
use bilbo_tantivy::{Committed, Options, Row, Tantivy};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

fn options() -> Options {
    let section = bilbo_search::keys::defaults();
    Options {
        memory: section.memory(),
        merge_deleted_share: section.merge_deleted_share(),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Bazel gives each test its scratch folder in TEST_TMPDIR"
)]
fn base() -> PathBuf {
    std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

impl Scratch {
    fn new(test: &str) -> Scratch {
        let dir = base().canonicalize().expect("a scratch base").join(format!(
            "{test}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create scratch");
        Scratch(dir)
    }

    fn folder(&self) -> PathBuf {
        bilbo_tantivy::folder(&self.0)
    }

    fn index(&self) -> Tantivy {
        Tantivy::at(self.folder(), options())
    }

    fn fresh(&self) -> Tantivy {
        self.fresh_with(options())
    }

    fn fresh_with(&self, options: Options) -> Tantivy {
        let mut index = Tantivy::at(self.folder(), options);
        index.clear().expect("clear");
        index
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn id(n: u8) -> Id {
    let mut bytes = [0; 16];
    bytes[15] = n;
    Id::from_bytes(bytes)
}

fn position(seq: u64) -> Position {
    Position {
        manifest: Manifest {
            chunk: Size {
                tokens: 512,
                overlap: 64,
            },
            analyzer: "bm25s-en/2".to_owned(),
            tokenizer: "qwen/1".to_owned(),
            render: "markdown/1/heading_width=100 session/1".to_owned(),
        },
        cursor: Cursor::at([9; 16], seq),
        failed: Vec::new(),
    }
}

fn chunk(first: u64, count: u64, part: Option<(u32, u32)>, text: &str) -> Chunk {
    Chunk {
        records: RecordRange { first, count },
        part,
        tokens: text.len(),
        text: text.to_owned(),
    }
}

fn add(index: &mut Tantivy, entry: u8, kind: &str, seconds: u64, chunk: &Chunk) {
    index
        .add(&Doc {
            entry: id(entry),
            generation: Generation(u32::from(entry) + 1),
            kind,
            date: UNIX_EPOCH + Duration::from_secs(seconds),
            chunk,
        })
        .expect("add a document");
}

const RUNNER: &str = "the runner was running in the garden";
const GARDENS: &str = "gardens and gardening tools";
const SHED: &str = "tools for the shed";
const ZEBRA: &str = "a zebra crossing by the garden";

fn row(
    entry: u8,
    kind: &str,
    seconds: i64,
    first: u64,
    count: u64,
    part: (u64, u64),
    text: &str,
) -> Row {
    Row {
        entry: id(entry).to_bytes(),
        generation: u64::from(entry) + 1,
        first,
        count,
        part: part.0,
        parts: part.1,
        date: seconds * 1000,
        kind: kind.to_owned(),
        length: analyze::terms(text).len() as u64,
        digest: chunk::digest(text),
    }
}

fn committed(scratch: &Scratch) -> Committed {
    Committed::at(scratch.folder())
}

fn rank(folder: &Path, text: &str, limit: usize) -> Vec<Candidate> {
    let committed = Committed::at(folder);
    let query = Query {
        text: text.to_owned(),
        kinds: Vec::new(),
    };
    Bm25 {
        index: &committed,
        k1: 1.2,
        b: 0.75,
        omit: Omit::default(),
        batch: 4096,
    }
    .rank(&query, limit)
    .expect("a rank")
}

fn key(candidate: &Candidate) -> (u64, u64) {
    let extent = &candidate.found.extent;
    (
        extent.records.first,
        extent.part.map_or(0, |(part, _)| u64::from(part)),
    )
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("list the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("a name")
        })
        .collect();
    names.sort();
    names
}

fn fill(index: &mut Tantivy) {
    add(index, 1, "session", 100, &chunk(1, 3, None, RUNNER));
    add(
        index,
        1,
        "session",
        100,
        &chunk(4, 1, Some((0, 2)), GARDENS),
    );
    add(index, 1, "session", 100, &chunk(4, 1, Some((1, 2)), SHED));
    add(index, 2, "markdown", 200, &chunk(7, 2, None, ZEBRA));
}

fn filled() -> Vec<Row> {
    vec![
        row(1, "session", 100, 1, 3, (0, 0), RUNNER),
        row(1, "session", 100, 4, 1, (0, 2), GARDENS),
        row(1, "session", 100, 4, 1, (1, 2), SHED),
        row(2, "markdown", 200, 7, 2, (0, 0), ZEBRA),
    ]
}

#[test]
fn a_missing_folder_becomes_a_private_index_that_holds_what_was_committed() {
    let scratch = Scratch::new("made");
    let mut index = scratch.index();
    assert!(
        !scratch.folder().exists(),
        "nothing is made before the first call"
    );
    assert_eq!(index.position().expect("a position"), None);
    index.clear().expect("clear");
    let mode = fs::metadata(scratch.folder())
        .expect("the folder")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
    fill(&mut index);
    let stats = index.commit(&position(5)).expect("commit");
    let on_disk: u64 = names(&scratch.folder())
        .iter()
        .map(|name| {
            fs::metadata(scratch.folder().join(name))
                .expect("a file")
                .len()
        })
        .sum();
    assert_eq!(stats.bytes, on_disk);
    assert_eq!(index.documents().expect("documents"), filled());
    assert_eq!(index.position().expect("a position"), Some(position(5)));
}

#[test]
fn another_process_finds_the_position_and_the_documents_of_the_last_commit() {
    let scratch = Scratch::new("reopened");
    let mut first = scratch.fresh();
    fill(&mut first);
    let mut left_out = position(5);
    left_out.failed = vec![(id(7), 1)];
    first.commit(&left_out).expect("commit");
    drop(first);
    let mut second = scratch.index();
    assert_eq!(second.position().expect("a position"), Some(left_out));
    assert_eq!(second.documents().expect("documents"), filled());
    add(
        &mut second,
        3,
        "session",
        300,
        &chunk(1, 1, None, "one more"),
    );
    second.commit(&position(6)).expect("commit");
    assert_eq!(second.documents().expect("documents").len(), 5);
    assert_eq!(
        scratch.index().position().expect("a position"),
        Some(position(6))
    );
}

#[test]
fn nothing_is_seen_and_nothing_stays_before_a_commit() {
    let scratch = Scratch::new("uncommitted");
    let mut first = scratch.fresh();
    fill(&mut first);
    first.commit(&position(5)).expect("commit");
    drop(first);
    let mut second = scratch.index();
    second.clear().expect("clear");
    add(
        &mut second,
        9,
        "session",
        900,
        &chunk(1, 1, None, "never committed"),
    );
    assert_eq!(second.documents().expect("documents"), filled());
    drop(second);
    let mut third = scratch.index();
    assert_eq!(third.position().expect("a position"), Some(position(5)));
    assert_eq!(third.documents().expect("documents"), filled());
}

#[test]
fn a_clear_and_the_documents_after_it_replace_the_index_in_one_commit() {
    let scratch = Scratch::new("replaced");
    let mut index = scratch.fresh();
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    index.clear().expect("clear");
    add(
        &mut index,
        3,
        "markdown",
        300,
        &chunk(2, 2, None, "the only one left"),
    );
    index.commit(&position(9)).expect("commit");
    assert_eq!(
        index.documents().expect("documents"),
        [row(3, "markdown", 300, 2, 2, (0, 0), "the only one left")]
    );
    assert_eq!(index.position().expect("a position"), Some(position(9)));
    index.clear().expect("clear");
    index.commit(&position(10)).expect("commit");
    assert_eq!(index.documents().expect("documents"), []);
}

#[test]
fn a_search_matches_the_analyzers_terms_and_ranks_by_how_often_and_how_rare() {
    let scratch = Scratch::new("searched");
    let mut index = scratch.fresh();
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    drop(index);
    let found = |text: &str| -> Vec<(u64, u64)> {
        let hits = rank(&scratch.folder(), text, 10);
        assert!(
            hits.windows(2).all(|pair| pair[0].score >= pair[1].score),
            "{hits:?}"
        );
        hits.iter().map(key).collect()
    };
    assert_eq!(found("runs"), [(1, 0)], "a stem matches its other forms");
    assert_eq!(
        found("GARDEN"),
        [(4, 0), (1, 0), (7, 0)],
        "the one that holds it twice, then two of one score in the order of their rows"
    );
    assert_eq!(found("zebra"), [(7, 0)]);
    assert_eq!(
        found("zebra tools"),
        [(7, 0), (4, 1), (4, 0)],
        "any term matches"
    );
    assert_eq!(found("the and of"), [], "a stopword is no term");
    assert_eq!(found("unknown"), []);
    assert_eq!(found(""), []);
    assert_eq!(rank(&scratch.folder(), "garden", 2).len(), 2);
    assert_eq!(rank(&scratch.folder(), "garden", 0).len(), 0);
}

#[test]
fn the_index_holds_no_text_of_a_chunk() {
    let scratch = Scratch::new("no-text");
    let mut index = scratch.fresh();
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    for name in names(&scratch.folder()) {
        let bytes = fs::read(scratch.folder().join(&name)).expect("a file");
        for phrase in ["zebra crossing", "running in", "the garden"] {
            let held = bytes
                .windows(phrase.len())
                .any(|window| window == phrase.as_bytes());
            assert!(!held, "{name} holds {phrase:?}");
        }
    }
}

#[test]
fn a_position_whose_names_hold_a_line_break_is_not_committed() {
    let scratch = Scratch::new("unencodable");
    let mut index = scratch.fresh();
    fill(&mut index);
    let mut broken = position(5);
    broken.manifest.analyzer = "two\nlines".to_owned();
    let error = index.commit(&broken).expect_err("no commit");
    assert!(
        matches!(
            error,
            IndexError::Failed {
                action: "commit",
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(index.position().expect("a position"), None);
    assert_eq!(index.documents().expect("documents"), []);
    let mut second = scratch.index();
    second
        .clear()
        .expect("the refused commit left no writer behind");
    second.commit(&position(5)).expect("commit");
    assert_eq!(second.position().expect("a position"), Some(position(5)));
    assert_eq!(second.documents().expect("documents"), []);
    drop(index);
    assert_eq!(scratch.index().documents().expect("documents"), []);
}

const MARKER: &[u8] = b"bilbo index\n";

type Snapshot = Vec<(String, String)>;

fn snapshot(folder: &Path) -> Snapshot {
    let mut found = Vec::new();
    let Ok(listed) = fs::read_dir(folder) else {
        return found;
    };
    for entry in listed {
        let entry = entry.expect("an entry");
        let path = entry.path();
        let kind = entry.file_type().expect("a type");
        let what = if kind.is_symlink() {
            format!("link {}", fs::read_link(&path).expect("a link").display())
        } else if kind.is_dir() {
            format!("dir {:?}", snapshot(&path))
        } else {
            format!("file {:?}", fs::read(&path).expect("a file"))
        };
        found.push((entry.file_name().into_string().expect("a name"), what));
    }
    found.sort();
    found
}

fn marker(folder: &Path) {
    fs::write(folder.join("bilbo-index"), MARKER).expect("write the marker");
}

fn built(folder: &Path) {
    let mut index = Tantivy::at(folder.to_path_buf(), options());
    index.clear().expect("clear");
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
}

fn refused(scratch: &Scratch, what: &str) {
    let before = snapshot(&scratch.0);
    let mut index = scratch.index();
    let reader = committed(scratch);
    let doc = Doc {
        entry: id(1),
        generation: Generation(1),
        kind: "session",
        date: UNIX_EPOCH,
        chunk: &chunk(1, 1, None, "text"),
    };
    for error in [
        index.position().map(|_| ()).expect_err("refused"),
        index.clear().expect_err("refused"),
        index.add(&doc).expect_err("refused"),
        index.commit(&position(1)).map(|_| ()).expect_err("refused"),
        index.documents().map(|_| ()).expect_err("refused"),
        reader.position().map(|_| ()).expect_err("refused"),
        reader.totals().map(|_| ()).expect_err("refused"),
        reader.postings("text").map(|_| ()).expect_err("refused"),
        reader
            .holding(&Filter::Kind("session"))
            .map(|_| ())
            .expect_err("refused"),
        reader.found(&[0]).map(|_| ()).expect_err("refused"),
        reader.documents().map(|_| ()).expect_err("refused"),
        reader.terms().map(|_| ()).expect_err("refused"),
    ] {
        assert!(matches!(error, IndexError::Foreign(_)), "{what}: {error}");
    }
    drop(index);
    assert_eq!(snapshot(&scratch.0), before, "{what}");
}

fn made_again(scratch: &Scratch, what: &str) {
    let mut index = scratch.index();
    index.clear().expect(what);
    fill(&mut index);
    index.commit(&position(5)).expect(what);
    assert_eq!(index.documents().expect(what), filled(), "{what}");
}

#[test]
fn an_index_folder_that_is_a_link_is_refused_and_what_it_points_at_is_untouched() {
    let target = Scratch::new("link-target");
    built(&target.folder());
    let scratch = Scratch::new("link");
    std::os::unix::fs::symlink(target.folder(), scratch.folder()).expect("link");
    let before = snapshot(&target.folder());
    refused(&scratch, "a link to a bilbo index");
    assert_eq!(snapshot(&target.folder()), before);
    assert!(
        fs::symlink_metadata(scratch.folder())
            .expect("the link")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_dangling_link_or_a_file_where_the_index_folder_should_be_is_refused() {
    let scratch = Scratch::new("dangling");
    std::os::unix::fs::symlink(scratch.0.join("nowhere"), scratch.folder()).expect("link");
    refused(&scratch, "a dangling link");
    assert!(!scratch.0.join("nowhere").exists());
    let scratch = Scratch::new("file-as-folder");
    fs::write(scratch.folder(), "mine").expect("write");
    refused(&scratch, "a file");
}

fn managed_with(folder: &Path, extra: &str) {
    let path = folder.join(".managed.json");
    let listed = fs::read_to_string(&path).expect(".managed.json");
    let open = listed.trim_end().strip_suffix(']').expect("a list");
    let comma = if open.trim_end().ends_with('[') {
        ""
    } else {
        ","
    };
    fs::write(&path, format!("{open}{comma}{extra}]")).expect("write");
}

#[test]
fn a_managed_list_that_names_a_path_outside_the_folder_deletes_nothing_outside_it() {
    for extra in ["\"../victim\"", "VICTIM", "\"sub/x\"", "\"..\""] {
        let scratch = Scratch::new("managed");
        built(&scratch.folder());
        let victim = scratch.0.join("victim");
        fs::write(&victim, "keep me").expect("write");
        let extra = extra.replace(
            "VICTIM",
            &format!("\"{}\"", victim.to_str().expect("a path")),
        );
        managed_with(&scratch.folder(), &extra);
        let mut index = scratch.index();
        index.clear().expect(&extra);
        add(
            &mut index,
            5,
            "session",
            500,
            &chunk(1, 1, None, "one text"),
        );
        index.commit(&position(6)).expect(&extra);
        assert_eq!(
            fs::read(&victim).expect("the victim"),
            b"keep me",
            "{extra}"
        );
        assert_eq!(
            index.documents().expect(&extra),
            [row(5, "session", 500, 1, 1, (0, 0), "one text")],
            "{extra}"
        );
    }
}

#[test]
fn a_managed_list_that_is_not_a_list_of_names_makes_the_index_again() {
    for content in ["{}", "[1]", "not json", ""] {
        let scratch = Scratch::new("managed-bad");
        built(&scratch.folder());
        fs::write(scratch.folder().join(".managed.json"), content).expect("write");
        made_again(&scratch, content);
    }
}

#[test]
fn a_folder_with_anything_in_it_and_no_marker_is_refused_and_left_as_it_is() {
    type Stray = (&'static str, fn(&Path));
    let strays: [Stray; 7] = [
        ("a file", |folder| {
            fs::write(folder.join("notes.txt"), "mine").expect("write");
        }),
        ("a folder", |folder| {
            fs::create_dir(folder.join("models")).expect("create");
        }),
        ("a temporary file", |folder| {
            fs::write(folder.join(".tmp-important-notes"), "mine").expect("write");
        }),
        ("a writer lock", |folder| {
            fs::write(folder.join(".tantivy-writer.lock"), "").expect("write");
        }),
        ("a segment file", |folder| {
            fs::write(folder.join("7827f006c8a44482bed98f5b97d6420f.idx"), "x").expect("write");
        }),
        ("a whole index without its marker", |folder| {
            built(folder);
            fs::remove_file(folder.join("bilbo-index")).expect("remove");
        }),
        ("a meta.json that is a link", |folder| {
            std::os::unix::fs::symlink("/etc/hosts", folder.join("meta.json")).expect("link");
        }),
    ];
    for (what, stray) in strays {
        let scratch = Scratch::new("foreign");
        fs::create_dir_all(scratch.folder()).expect("create");
        stray(&scratch.folder());
        refused(&scratch, what);
    }
}

#[test]
fn a_marker_with_other_bytes_or_of_another_kind_does_not_make_a_folder_bilbos() {
    type Marker = (&'static str, fn(&Scratch));
    let markers: [Marker; 5] = [
        ("no line feed and another file", |scratch| {
            fs::write(scratch.folder().join("bilbo-index"), "bilbo index").expect("write");
            fs::write(scratch.folder().join("meta.json"), "{}").expect("write");
        }),
        ("two line feeds", |scratch| {
            fs::write(scratch.folder().join("bilbo-index"), "bilbo index\n\n").expect("write");
        }),
        ("another case", |scratch| {
            fs::write(scratch.folder().join("bilbo-index"), "Bilbo index\n").expect("write");
        }),
        ("a link to the right bytes", |scratch| {
            fs::write(scratch.0.join("right"), MARKER).expect("write");
            std::os::unix::fs::symlink(
                scratch.0.join("right"),
                scratch.folder().join("bilbo-index"),
            )
            .expect("link");
        }),
        ("a folder", |scratch| {
            fs::create_dir(scratch.folder().join("bilbo-index")).expect("create");
        }),
    ];
    for (what, make) in markers {
        let scratch = Scratch::new("marker-kind");
        fs::create_dir_all(scratch.folder()).expect("create");
        make(&scratch);
        refused(&scratch, what);
    }
}

#[test]
fn a_marked_folder_with_a_stranger_in_it_is_refused_and_left_as_it_is() {
    type Stranger = (&'static str, fn(&Path));
    let strangers: [Stranger; 3] = [
        ("a file", |folder| {
            fs::write(folder.join("notes.txt"), "mine").expect("write");
        }),
        ("a subfolder", |folder| {
            fs::create_dir(folder.join("models")).expect("create");
        }),
        ("a link named like a segment file", |folder| {
            std::os::unix::fs::symlink(
                "/etc/hosts",
                folder.join("7827f006c8a44482bed98f5b97d6420f.idx"),
            )
            .expect("link");
        }),
    ];
    for (what, stranger) in strangers {
        let scratch = Scratch::new("stranger");
        built(&scratch.folder());
        stranger(&scratch.folder());
        refused(&scratch, what);
    }
}

#[test]
fn a_new_index_has_its_marker_before_any_other_file() {
    let scratch = Scratch::new("marker-first");
    let marked = |when: &str| {
        let path = scratch.folder().join("bilbo-index");
        assert_eq!(fs::read(&path).expect(when), MARKER, "{when}");
        let mode = fs::metadata(&path).expect(when).permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{when}");
    };
    let mut index = scratch.index();
    index.clear().expect("clear");
    marked("after the first call");
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    marked("after commit");
    let mut again = scratch.index();
    again.clear().expect("clear");
    marked("after reopening");
}

#[test]
fn an_index_interrupted_at_any_point_is_made_again_or_used() {
    type Left = (&'static str, fn(&Path));
    let lefts: [Left; 7] = [
        ("a truncated marker alone", |folder| {
            fs::write(folder.join("bilbo-index"), "bilbo i").expect("write");
        }),
        ("an empty marker alone", |folder| {
            fs::write(folder.join("bilbo-index"), "").expect("write");
        }),
        ("the marker alone", marker),
        ("the marker, a temporary file and a lock", |folder| {
            marker(folder);
            fs::write(folder.join(".tmp1"), "").expect("write");
            fs::write(folder.join(".tantivy-writer.lock"), "").expect("write");
        }),
        ("an index made and never committed", |folder| {
            let mut index = Tantivy::at(folder.to_path_buf(), options());
            index.clear().expect("clear");
            fill(&mut index);
        }),
        (
            "a committed index with segment files it does not list",
            |folder| {
                built(folder);
                fs::write(folder.join("7827f006c8a44482bed98f5b97d6420f.idx"), "x").expect("write");
                fs::write(folder.join("7827f006c8a44482bed98f5b97d6420f.pos"), "x").expect("write");
            },
        ),
        ("a cleanup cut short", |folder| {
            marker(folder);
            fs::write(folder.join(".managed.json"), "[]").expect("write");
            fs::write(folder.join("7827f006c8a44482bed98f5b97d6420f.term"), "x").expect("write");
        }),
    ];
    for (what, leave) in lefts {
        let scratch = Scratch::new("interrupted");
        fs::create_dir_all(scratch.folder()).expect("create");
        leave(&scratch.folder());
        made_again(&scratch, what);
    }
}

#[test]
fn a_marked_index_of_another_schema_or_one_tantivy_cannot_open_is_made_again() {
    let scratch = Scratch::new("older");
    fs::create_dir_all(scratch.folder()).expect("create");
    marker(&scratch.folder());
    let mut builder = tantivy::schema::Schema::builder();
    builder.add_text_field("text", tantivy::schema::TEXT);
    let older = tantivy::Index::create_in_dir(scratch.folder(), builder.build()).expect("an index");
    let mut writer: tantivy::IndexWriter = older.writer(15_000_000).expect("a writer");
    let mut prepared = writer.prepare_commit().expect("prepare");
    prepared.set_payload("bilbo index 0\nanything\n");
    prepared.commit().expect("commit");
    drop(writer);
    drop(older);
    let mut index = scratch.index();
    assert_eq!(index.position().expect("a position"), None);
    drop(index);
    made_again(&scratch, "another schema");

    let scratch = Scratch::new("unreadable");
    built(&scratch.folder());
    fs::write(scratch.folder().join("meta.json"), "not json").expect("write");
    made_again(&scratch, "a meta.json tantivy cannot read");
}

#[test]
fn an_index_made_but_never_committed_is_bilbos_to_fill() {
    let scratch = Scratch::new("half-built");
    let mut first = scratch.index();
    first.clear().expect("clear");
    fill(&mut first);
    drop(first);
    assert!(names(&scratch.folder()).contains(&"meta.json".to_owned()));
    let mut second = scratch.index();
    assert_eq!(second.position().expect("a position"), None);
    assert_eq!(second.documents().expect("documents"), []);
    fill(&mut second);
    second.commit(&position(5)).expect("commit");
    assert_eq!(second.documents().expect("documents").len(), 4);
}

#[test]
fn a_second_writer_on_the_folder_fails_and_says_so() {
    let scratch = Scratch::new("two-writers");
    let mut first = scratch.index();
    first.clear().expect("clear");
    let mut second = scratch.index();
    let error = second.clear().expect_err("the folder has a writer");
    assert!(
        matches!(
            error,
            IndexError::Failed {
                action: "write",
                ..
            }
        ),
        "{error}"
    );
    drop(first);
    scratch
        .index()
        .clear()
        .expect("the lock went with its writer");
}

#[test]
fn two_indexes_of_the_same_documents_have_the_same_rows_and_the_same_hits_whatever_the_order_of_their_segments()
 {
    let one = Scratch::new("same-1");
    let two = Scratch::new("same-2");
    let mut first = one.fresh();
    fill(&mut first);
    first.commit(&position(5)).expect("commit");
    let mut second = two.fresh();
    add(
        &mut second,
        9,
        "session",
        900,
        &chunk(1, 1, None, "gone before the end"),
    );
    second.commit(&position(1)).expect("commit");
    second.clear().expect("clear");
    fill(&mut second);
    second.commit(&position(5)).expect("commit");
    assert_eq!(
        first.documents().expect("documents"),
        second.documents().expect("documents")
    );
    for query in ["garden", "zebra tools", "running shed", "nothing"] {
        for limit in [1, 2, 10] {
            assert_eq!(
                rank(&one.folder(), query, limit),
                rank(&two.folder(), query, limit),
                "{query} {limit}"
            );
        }
    }
}

#[test]
fn of_documents_with_one_score_the_lowest_rows_are_kept_whatever_order_they_were_added_in() {
    let same = || chunk(1, 1, None, "same words here");
    let rising = Scratch::new("ties-rising");
    let mut first = rising.fresh();
    for entry in 1..=4 {
        add(&mut first, entry, "session", 100, &same());
    }
    first.commit(&position(5)).expect("commit");
    let falling = Scratch::new("ties-falling");
    let mut second = falling.fresh();
    for entry in (1..=4).rev() {
        add(&mut second, entry, "session", 100, &same());
        second.commit(&position(5)).expect("commit");
    }
    for limit in 1..=4 {
        let entries = |scratch: &Scratch| -> Vec<u8> {
            rank(&scratch.folder(), "same words", limit)
                .iter()
                .map(|hit| hit.found.entry.to_bytes()[15])
                .collect()
        };
        let wanted: Vec<u8> = (1..=4).take(limit).collect();
        assert_eq!(entries(&rising), wanted, "rising, limit {limit}");
        assert_eq!(entries(&falling), wanted, "falling, limit {limit}");
    }
}

#[test]
fn a_document_is_found_by_its_longest_term_and_a_longer_word_is_in_no_document() {
    let scratch = Scratch::new("longest");
    let mut index = scratch.fresh();
    let longest = "x".repeat(bilbo_search::analyze::MAX_TERM_BYTES);
    let longer = "y".repeat(bilbo_search::analyze::MAX_TERM_BYTES + 1);
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(1, 1, None, &format!("{longest} {longer} zebra")),
    );
    index.commit(&position(5)).expect("commit");
    assert_eq!(rank(&scratch.folder(), &longest, 10).len(), 1);
    assert_eq!(rank(&scratch.folder(), &longer, 10).len(), 0);
    assert_eq!(rank(&scratch.folder(), "zebra", 10).len(), 1);

    let opened = tantivy::Index::open_in_dir(scratch.folder()).expect("the index");
    let field = opened.schema().get_field("text").expect("the text field");
    let searcher = opened.reader().expect("a reader").searcher();
    let mut lengths = Vec::new();
    for segment in searcher.segment_readers() {
        let inverted = segment
            .inverted_index(field)
            .expect("the text field's terms");
        let mut terms = inverted.terms().stream().expect("the terms");
        while terms.advance() {
            lengths.push(terms.key().len());
        }
    }
    assert!(!lengths.is_empty());
    assert_eq!(
        lengths.iter().max(),
        Some(&bilbo_search::analyze::MAX_TERM_BYTES),
        "the longest term is the longest word that is kept, and no word is longer"
    );
}

#[test]
fn the_text_field_keeps_frequencies_and_no_positions() {
    let scratch = Scratch::new("freq");
    let mut index = scratch.index();
    index.clear().expect("clear");
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    let meta = fs::read_to_string(scratch.folder().join("meta.json")).expect("meta.json");
    let from = meta.find(r#""name": "text""#).expect("the text field");
    let field = &meta[from..];
    let field = &field[..field[1..]
        .find(r#""name":"#)
        .map_or(field.len(), |end| end + 1)];
    assert!(field.contains(r#""record": "freq""#), "{field}");
    assert!(field.contains(r#""stored": false"#), "{field}");
    for name in names(&scratch.folder()) {
        if name.ends_with(".pos") {
            let size = fs::metadata(scratch.folder().join(&name))
                .expect("file")
                .len();
            assert!(size < 500, "{name} is {size} bytes");
        }
    }
}

fn extent(
    generation: u32,
    first: u64,
    count: u64,
    part: Option<(u32, u32)>,
    seconds: i64,
) -> Extent {
    Extent {
        generation: Generation(generation),
        records: RecordRange { first, count },
        part,
        date: seconds * 1000,
    }
}

type Left = (&'static str, fn(&Path));

fn unusable() -> [Left; 7] {
    [
        ("a missing folder", |_| {}),
        ("an empty folder", |folder| {
            fs::create_dir_all(folder).expect("create");
        }),
        ("a truncated marker alone", |folder| {
            fs::create_dir_all(folder).expect("create");
            fs::write(folder.join("bilbo-index"), "bilbo i").expect("write");
        }),
        ("the marker alone", |folder| {
            fs::create_dir_all(folder).expect("create");
            marker(folder);
        }),
        ("a meta.json tantivy cannot read", |folder| {
            built(folder);
            fs::write(folder.join("meta.json"), "not json").expect("write");
        }),
        ("segment files and a lock with no meta.json", |folder| {
            fs::create_dir_all(folder).expect("create");
            marker(folder);
            fs::write(folder.join("7827f006c8a44482bed98f5b97d6420f.idx"), "x").expect("write");
            fs::write(folder.join(".tantivy-writer.lock"), "").expect("write");
        }),
        ("another schema", |folder| {
            fs::create_dir_all(folder).expect("create");
            marker(folder);
            let mut builder = tantivy::schema::Schema::builder();
            builder.add_text_field("text", tantivy::schema::TEXT);
            let older = tantivy::Index::create_in_dir(folder, builder.build()).expect("an index");
            let mut writer: tantivy::IndexWriter = older.writer(15_000_000).expect("a writer");
            let mut prepared = writer.prepare_commit().expect("prepare");
            prepared.set_payload("bilbo index 0\nanything\n");
            prepared.commit().expect("commit");
        }),
    ]
}

#[test]
fn asking_the_position_of_a_missing_a_broken_or_an_older_index_changes_no_file() {
    let lefts = unusable();
    for (what, leave) in lefts {
        let scratch = Scratch::new("read-only");
        leave(&scratch.folder());
        let before = snapshot(&scratch.0);
        let mut index = scratch.index();
        assert_eq!(index.position().expect(what), None, "{what}");
        assert_eq!(index.extents(id(1)).expect(what), [], "{what}");
        drop(index);
        assert_eq!(snapshot(&scratch.0), before, "{what}");
    }
}

#[test]
fn asking_the_position_or_the_extents_of_a_committed_index_changes_no_file_and_takes_no_writer() {
    let scratch = Scratch::new("read-committed");
    built(&scratch.folder());
    let before = snapshot(&scratch.0);
    let mut reader = scratch.index();
    assert_eq!(reader.position().expect("a position"), Some(position(5)));
    assert_eq!(reader.extents(id(1)).expect("extents").len(), 3);
    assert_eq!(snapshot(&scratch.0), before);
    let mut writer = scratch.index();
    writer.clear().expect("the reader holds no writer");
}

#[test]
fn the_extents_of_an_entry_are_its_documents_of_the_last_commit_in_chunk_order() {
    let scratch = Scratch::new("extents");
    let mut index = scratch.fresh();
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(4, 1, Some((1, 2)), "tools for the shed"),
    );
    add(
        &mut index,
        2,
        "markdown",
        200,
        &chunk(7, 2, None, "a zebra crossing"),
    );
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(1, 3, None, "the runner"),
    );
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(4, 1, Some((0, 2)), "gardens and tools"),
    );
    index.commit(&position(1)).expect("commit");
    assert_eq!(
        index.extents(id(1)).expect("extents"),
        [
            extent(2, 1, 3, None, 100),
            extent(2, 4, 1, Some((0, 2)), 100),
            extent(2, 4, 1, Some((1, 2)), 100),
        ]
    );
    assert_eq!(
        index.extents(id(2)).expect("extents"),
        [extent(3, 7, 2, None, 200)]
    );
    assert_eq!(index.extents(id(3)).expect("extents"), []);

    add(&mut index, 1, "session", 100, &chunk(9, 1, None, "pending"));
    index.delete(id(1), 1).expect("delete");
    assert_eq!(
        index.extents(id(1)).expect("extents").len(),
        3,
        "only the last commit is read"
    );
    index.rollback();
}

#[test]
fn a_delete_from_a_record_takes_the_documents_from_it_on_and_no_other_entrys() {
    let scratch = Scratch::new("delete-from");
    built(&scratch.folder());
    let mut index = scratch.index();
    index.delete(id(1), 4).expect("delete");
    index.delete(id(2), 8).expect("delete");
    index.commit(&position(6)).expect("commit");
    assert_eq!(
        index.documents().expect("documents"),
        [
            row(1, "session", 100, 1, 3, (0, 0), RUNNER),
            row(2, "markdown", 200, 7, 2, (0, 0), ZEBRA),
        ],
        "an earlier commit's documents from record 4 on; entry 2 starts at 7, before 8"
    );

    let mut index = scratch.index();
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(10, 1, None, "added before the delete"),
    );
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(20, 1, None, "added before the delete too"),
    );
    index.delete(id(1), 15).expect("delete");
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(30, 1, None, "added after the delete"),
    );
    index.commit(&position(7)).expect("commit");
    assert_eq!(
        index.documents().expect("documents"),
        [
            row(1, "session", 100, 1, 3, (0, 0), RUNNER),
            row(1, "session", 100, 10, 1, (0, 0), "added before the delete"),
            row(1, "session", 100, 30, 1, (0, 0), "added after the delete"),
            row(2, "markdown", 200, 7, 2, (0, 0), ZEBRA),
        ]
    );

    let mut index = scratch.index();
    index.delete(id(1), 1).expect("delete all");
    index.commit(&position(8)).expect("commit");
    assert_eq!(
        index.documents().expect("documents"),
        [row(2, "markdown", 200, 7, 2, (0, 0), ZEBRA)]
    );
}

#[test]
fn a_delete_and_adds_in_one_commit_replace() {
    let scratch = Scratch::new("replace");
    built(&scratch.folder());
    let mut index = scratch.index();
    index.delete(id(1), 4).expect("delete");
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(4, 2, None, "the shed and the tools"),
    );
    index.commit(&position(6)).expect("commit");

    let rebuilt = Scratch::new("replace-rebuilt");
    let mut fresh = rebuilt.fresh();
    add(
        &mut fresh,
        1,
        "session",
        100,
        &chunk(1, 3, None, "the runner was running in the garden"),
    );
    add(
        &mut fresh,
        1,
        "session",
        100,
        &chunk(4, 2, None, "the shed and the tools"),
    );
    add(
        &mut fresh,
        2,
        "markdown",
        200,
        &chunk(7, 2, None, "a zebra crossing by the garden"),
    );
    fresh.commit(&position(6)).expect("commit");

    assert_eq!(
        index.documents().expect("documents"),
        fresh.documents().expect("documents")
    );
    assert_eq!(index.terms().expect("terms"), fresh.terms().expect("terms"));
}

#[test]
fn a_rollback_drops_what_was_added_and_deleted_and_frees_the_writer() {
    let scratch = Scratch::new("rollback");
    built(&scratch.folder());
    let mut index = scratch.index();
    index.delete(id(1), 1).expect("delete");
    add(
        &mut index,
        3,
        "session",
        300,
        &chunk(1, 1, None, "never committed"),
    );
    index.rollback();

    let mut other = scratch.index();
    other
        .delete(id(2), 1)
        .expect("a second adapter can write once the first rolled back");
    other.rollback();
    assert_eq!(index.documents().expect("documents"), filled());

    add(
        &mut index,
        3,
        "session",
        300,
        &chunk(1, 1, None, "committed after a rollback"),
    );
    index.commit(&position(6)).expect("commit");
    assert_eq!(index.documents().expect("documents").len(), 5);
    let mut again = scratch.index();
    again.clear().expect("a commit frees the writer too");
}

fn same_scores(grown: &Scratch, rebuilt: &Scratch, queries: &[&str]) {
    for query in queries {
        for limit in [1, 5, 50] {
            let grown_hits = rank(&grown.folder(), query, limit);
            let rebuilt_hits = rank(&rebuilt.folder(), query, limit);
            assert_eq!(grown_hits, rebuilt_hits, "{query} {limit}");
        }
    }
    let (grown_totals, rebuilt_totals) = (
        committed(grown).totals().expect("totals"),
        committed(rebuilt).totals().expect("totals"),
    );
    assert_eq!(
        (
            grown_totals.documents,
            grown_totals.entries,
            grown_totals.length
        ),
        (
            rebuilt_totals.documents,
            rebuilt_totals.entries,
            rebuilt_totals.length
        ),
        "the statistics count live documents only"
    );
}

#[test]
fn a_grown_index_ranks_as_a_rebuilt_one_scores_included() {
    let one = |word: &str| chunk(1, 1, None, word);
    let word = |entry: u8| if entry <= 20 { "zebra" } else { "apple" };
    let grown = Scratch::new("grown");
    let mut index = grown.fresh();
    for entry in 1..=100 {
        add(&mut index, entry, "session", 100, &one(word(entry)));
    }
    index.commit(&position(1)).expect("commit");
    for entry in 1..=19 {
        index.delete(id(entry), 1).expect("delete");
        add(&mut index, entry, "session", 100, &one("yak"));
    }
    index.commit(&position(2)).expect("commit");

    let rebuilt = Scratch::new("rebuilt");
    let mut fresh = rebuilt.fresh();
    for entry in 1..=100 {
        let text = if entry <= 19 { "yak" } else { word(entry) };
        add(&mut fresh, entry, "session", 100, &one(text));
    }
    fresh.commit(&position(2)).expect("commit");

    assert_eq!(
        index.documents().expect("documents"),
        fresh.documents().expect("documents")
    );
    assert_eq!(index.terms().expect("terms"), fresh.terms().expect("terms"));
    assert_eq!(rank(&grown.folder(), "zebra", 10).len(), 1);
    same_scores(
        &grown,
        &rebuilt,
        &["zebra", "yak", "apple", "zebra yak apple"],
    );
}

#[test]
fn a_grown_session_ranks_as_a_rebuilt_one_scores_included() {
    let first = [
        (1, 2, "the first question about the garden"),
        (3, 1, "an answer about a zebra and the garden"),
    ];
    let later = [
        (4, 1, "a later question about tools"),
        (5, 3, "the shed holds tools and more tools and the garden"),
    ];
    let other = chunk(1, 1, None, "a note on the garden and the shed");
    let grown = Scratch::new("grown-session");
    let mut index = grown.fresh();
    for (at, count, text) in first {
        add(&mut index, 1, "session", 100, &chunk(at, count, None, text));
    }
    add(&mut index, 2, "markdown", 200, &other);
    index.commit(&position(1)).expect("commit");
    for (at, count, text) in later {
        add(&mut index, 1, "session", 100, &chunk(at, count, None, text));
    }
    index.commit(&position(2)).expect("commit");

    let rebuilt = Scratch::new("rebuilt-session");
    let mut fresh = rebuilt.fresh();
    for (at, count, text) in first.into_iter().chain(later) {
        add(&mut fresh, 1, "session", 100, &chunk(at, count, None, text));
    }
    add(&mut fresh, 2, "markdown", 200, &other);
    fresh.commit(&position(2)).expect("commit");

    same_scores(
        &grown,
        &rebuilt,
        &["garden", "tools", "zebra shed", "question"],
    );
}

fn deletes_left_after(options: Options, deleted: u8) -> (usize, usize) {
    let scratch = Scratch::new("merge-share");
    let mut index = scratch.fresh_with(options);
    for entry in 1..=100 {
        add(
            &mut index,
            entry,
            "session",
            100,
            &chunk(1, 1, None, "word"),
        );
    }
    index.commit(&position(1)).expect("commit");
    for entry in 1..=deleted {
        index.delete(id(entry), 1).expect("delete");
    }
    index.commit(&position(2)).expect("commit");
    let deletes = names(&scratch.folder())
        .iter()
        .filter(|name| name.ends_with(".del"))
        .count();
    (deletes, index.documents().expect("documents").len())
}

#[test]
fn a_commit_merges_away_a_segment_with_over_a_fifth_of_its_documents_deleted() {
    assert_eq!(
        deletes_left_after(options(), 19),
        (1, 81),
        "19 of 100 is under the share: the deletes stay"
    );
    assert_eq!(
        deletes_left_after(options(), 22),
        (0, 78),
        "22 of 100 is over it: the segment was merged"
    );
    let never = Options {
        merge_deleted_share: 1.0,
        ..options()
    };
    assert_eq!(
        deletes_left_after(never, 22),
        (1, 78),
        "a share of 1.0 keeps the delete file"
    );
}

#[test]
fn a_writer_takes_the_memory_it_is_given() {
    let small = Options {
        memory: 16 * 1024 * 1024,
        ..options()
    };
    assert_eq!(deletes_left_after(small, 1), (1, 99));
    let scratch = Scratch::new("memory");
    let mut index = Tantivy::at(
        scratch.folder(),
        Options {
            memory: 1024 * 1024,
            ..options()
        },
    );
    let error = index.clear().expect_err("under tantivy's minimum");
    assert!(
        error.to_string().contains("memory arena"),
        "tantivy's own error: {error}"
    );
}

#[test]
fn only_a_clear_makes_or_repairs_the_index_folder() {
    type State = (&'static str, fn(&Path));
    let states: [State; 8] = [
        ("an absent folder", |_| {}),
        ("an empty folder", |folder| {
            fs::create_dir_all(folder).expect("create");
        }),
        ("a marker cut short", |folder| {
            fs::create_dir_all(folder).expect("create");
            fs::write(folder.join("bilbo-index"), "bilbo i").expect("write");
        }),
        ("the marker alone", |folder| {
            fs::create_dir_all(folder).expect("create");
            marker(folder);
        }),
        ("a meta.json that is not json", |folder| {
            built(folder);
            fs::write(folder.join("meta.json"), "not json").expect("write");
        }),
        ("a meta.json that is gone", |folder| {
            built(folder);
            fs::remove_file(folder.join("meta.json")).expect("remove");
        }),
        ("a managed list that is not a list", |folder| {
            built(folder);
            fs::write(folder.join(".managed.json"), "{}").expect("write");
        }),
        ("a segment file that is gone", |folder| {
            built(folder);
            let idx = names(folder)
                .into_iter()
                .find(|name| name.ends_with(".idx"))
                .expect("a postings file");
            fs::remove_file(folder.join(idx)).expect("remove");
        }),
    ];
    for (what, leave) in states {
        let scratch = Scratch::new("only-clear");
        leave(&scratch.folder());
        let before = snapshot(&scratch.0);
        let doc = Doc {
            entry: id(1),
            generation: Generation(1),
            kind: "session",
            date: UNIX_EPOCH,
            chunk: &chunk(1, 1, None, "text"),
        };
        for error in [
            scratch.index().add(&doc).expect_err(what),
            scratch.index().delete(id(1), 1).expect_err(what),
            scratch
                .index()
                .commit(&position(1))
                .map(|_| ())
                .expect_err(what),
        ] {
            assert!(
                matches!(error, IndexError::Failed { .. }),
                "{what}: {error}"
            );
        }
        assert_eq!(snapshot(&scratch.0), before, "{what}");
        made_again(&scratch, what);
    }
}

#[test]
fn a_committed_index_with_a_segment_file_missing_or_cut_has_no_position_and_the_next_clear_remakes_it()
 {
    let reference = Scratch::new("segments-reference");
    let mut wanted = reference.fresh();
    fill(&mut wanted);
    wanted.commit(&position(5)).expect("commit");
    for extension in [".fast", ".idx", ".term", ".store", ".fieldnorm"] {
        type Damage = (&'static str, fn(&Path));
        let damages: [Damage; 3] = [
            ("gone", |file| fs::remove_file(file).expect("remove")),
            ("a byte short", |file| {
                let mut bytes = fs::read(file).expect("read");
                bytes.pop();
                fs::write(file, bytes).expect("write");
            }),
            ("empty", |file| fs::write(file, b"").expect("write")),
        ];
        for (how, damage) in damages {
            let what = format!("{extension} {how}");
            let scratch = Scratch::new("segments");
            built(&scratch.folder());
            let file = names(&scratch.folder())
                .into_iter()
                .find(|name| name.ends_with(extension))
                .expect("the segment's file");
            damage(&scratch.folder().join(file));
            let before = snapshot(&scratch.0);
            let mut index = scratch.index();
            assert_eq!(index.position().expect(&what), None, "{what}");
            assert_eq!(index.extents(id(1)).expect(&what), [], "{what}");
            assert_eq!(snapshot(&scratch.0), before, "{what}");
            index.clear().expect(&what);
            fill(&mut index);
            index.commit(&position(5)).expect(&what);
            assert_eq!(index.position().expect(&what), Some(position(5)), "{what}");
            assert_eq!(
                index.terms().expect(&what),
                wanted.terms().expect(&what),
                "{what}"
            );
        }
    }
}

fn built_with_a_delete_file(folder: &Path) {
    let mut index = Tantivy::at(folder.to_path_buf(), options());
    index.clear().expect("clear");
    for entry in 1..=100 {
        add(
            &mut index,
            entry,
            "session",
            100,
            &chunk(1, 1, None, "word"),
        );
    }
    index.commit(&position(1)).expect("commit");
    index.delete(id(1), 1).expect("delete");
    index.commit(&position(5)).expect("commit");
    assert!(names(folder).iter().any(|name| name.ends_with(".del")));
}

fn component(folder: &Path, extension: &str) -> PathBuf {
    folder.join(
        names(folder)
            .into_iter()
            .find(|name| name.ends_with(extension))
            .expect("the segment's file"),
    )
}

/// The first byte of the version that an `SSTable` in the `.fast` file of `fill`'s index starts
/// with: tantivy's parser rejects another version with an error that is not one of a broken file.
const SSTABLE_VERSION_AT: usize = 413;

fn sstable_version_changed(file: &Path) {
    let mut bytes = fs::read(file).expect("read");
    say(&format!(
        "the .fast file is {} bytes; its bytes at {SSTABLE_VERSION_AT}..+4 are {:?}",
        bytes.len(),
        &bytes[SSTABLE_VERSION_AT..SSTABLE_VERSION_AT + 4]
    ));
    bytes[SSTABLE_VERSION_AT] ^= 0x55;
    fs::write(file, bytes).expect("write");
}

#[test]
fn a_component_cut_to_fewer_bytes_than_its_footer_is_a_broken_index_and_never_a_panic() {
    let reference = Scratch::new("footer-reference");
    let mut wanted = reference.fresh();
    fill(&mut wanted);
    wanted.commit(&position(5)).expect("commit");
    for extension in [".idx", ".fast", ".term", ".store", ".fieldnorm", ".del"] {
        for length in 4..8 {
            let what = format!("{extension} cut to {length} bytes");
            let scratch = Scratch::new("footer");
            if extension == ".del" {
                built_with_a_delete_file(&scratch.folder());
            } else {
                built(&scratch.folder());
            }
            let file = component(&scratch.folder(), extension);
            let mut bytes = fs::read(&file).expect("read");
            bytes.truncate(length);
            fs::write(&file, bytes).expect("write");
            let before = snapshot(&scratch.0);
            let mut index = scratch.index();
            assert_eq!(index.position().expect(&what), None, "{what}");
            assert_eq!(index.extents(id(1)).expect(&what), [], "{what}");
            assert_eq!(snapshot(&scratch.0), before, "{what}");
            index.clear().expect(&what);
            fill(&mut index);
            index.commit(&position(5)).expect(&what);
            assert_eq!(
                index.terms().expect(&what),
                wanted.terms().expect(&what),
                "{what}"
            );
        }
    }
}

#[test]
fn a_clear_remakes_an_owned_index_whatever_its_segments_hold() {
    let reference = Scratch::new("whatever-reference");
    let mut wanted = reference.fresh();
    fill(&mut wanted);
    wanted.commit(&position(5)).expect("commit");
    let scratch = Scratch::new("whatever");
    built(&scratch.folder());
    sstable_version_changed(&component(&scratch.folder(), ".fast"));
    let before = snapshot(&scratch.0);
    let mut index = scratch.index();
    let error = index.position().expect_err("a position cannot be read");
    say(&format!(
        "damage: the SSTable version in .fast; position() gave: {error}"
    ));
    assert!(matches!(error, IndexError::Failed { .. }), "{error}");
    assert_eq!(snapshot(&scratch.0), before);
    index.clear().expect("clear");
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    assert_eq!(index.position().expect("a position"), Some(position(5)));
    assert_eq!(
        index.terms().expect("terms"),
        wanted.terms().expect("terms")
    );
}

#[test]
fn a_clear_never_reads_a_component_file() {
    let reference = Scratch::new("never-reference");
    let mut wanted = reference.fresh();
    fill(&mut wanted);
    wanted.commit(&position(5)).expect("commit");
    let scratch = Scratch::new("never");
    built_with_a_delete_file(&scratch.folder());
    let folder = scratch.folder();
    let components: Vec<PathBuf> = names(&folder)
        .into_iter()
        .filter(|name| name.len() > 32 && name.as_bytes()[32] == b'.')
        .map(|name| folder.join(name))
        .collect();
    assert!(components.len() >= 6);
    for file in &components {
        fs::write(file, b"").expect("empty");
        fs::set_permissions(file, fs::Permissions::from_mode(0o000)).expect("chmod");
    }
    let readable_anyway = fs::read(&components[0]).is_ok();
    let mut index = scratch.index();
    let outcome = index.clear().and_then(|()| {
        fill(&mut index);
        index.commit(&position(5)).map(|_| ())
    });
    for file in &components {
        let _ = fs::set_permissions(file, fs::Permissions::from_mode(0o600));
    }
    if readable_anyway {
        say("skipped: the test's user reads a file of mode 000");
        return;
    }
    outcome.expect("clear, adds and a commit read no component");
    assert_eq!(
        index.terms().expect("terms"),
        wanted.terms().expect("terms")
    );
}

#[test]
fn an_index_that_cannot_be_opened_for_another_reason_is_an_error_and_not_an_empty_index() {
    let scratch = Scratch::new("unreadable-meta");
    built(&scratch.folder());
    let meta = scratch.folder().join("meta.json");
    let before = snapshot(&scratch.0);
    fs::set_permissions(&meta, fs::Permissions::from_mode(0o000)).expect("chmod");
    let readable_anyway = fs::read(&meta).is_ok();
    let mut index = scratch.index();
    let position = index.position();
    let extents = index.extents(id(1));
    let cleared = index.clear();
    let added = scratch.index().add(&Doc {
        entry: id(1),
        generation: Generation(1),
        kind: "session",
        date: UNIX_EPOCH,
        chunk: &chunk(1, 1, None, "text"),
    });
    drop(index);
    fs::set_permissions(&meta, fs::Permissions::from_mode(0o600)).expect("chmod");
    if readable_anyway {
        say("skipped: the test's user reads a file of mode 000");
        return;
    }
    for (what, error) in [
        ("position", position.map(|_| ()).expect_err("position")),
        ("extents", extents.map(|_| ()).expect_err("extents")),
        ("clear", cleared.expect_err("clear")),
        ("add", added.expect_err("add")),
    ] {
        assert!(
            matches!(error, IndexError::Failed { .. }),
            "{what}: {error}"
        );
    }
    assert_eq!(snapshot(&scratch.0), before);
}

#[test]
fn two_entries_at_the_same_generation_and_records_keep_their_own_terms() {
    let put = |index: &mut Tantivy, entry: u8, text: &str| {
        index
            .add(&Doc {
                entry: id(entry),
                generation: Generation(1),
                kind: "session",
                date: UNIX_EPOCH,
                chunk: &chunk(1, 1, None, text),
            })
            .expect("add");
    };
    let grown = Scratch::new("same-coordinates");
    let mut index = grown.fresh();
    put(&mut index, 1, "alpha beta");
    put(&mut index, 2, "gamma delta");
    index.commit(&position(1)).expect("commit");
    index.delete(id(1), 1).expect("delete");
    put(&mut index, 1, "epsilon zeta");
    index.commit(&position(2)).expect("commit");

    let rebuilt = Scratch::new("same-coordinates-rebuilt");
    let mut fresh = rebuilt.fresh();
    put(&mut fresh, 1, "epsilon zeta");
    put(&mut fresh, 2, "gamma delta");
    fresh.commit(&position(2)).expect("commit");

    let terms = index.terms().expect("terms");
    assert_eq!(terms, fresh.terms().expect("terms"));
    assert_eq!(terms.len(), 2);
    assert_ne!(terms[0].1, terms[1].1);
    assert_eq!(terms[0].0.entry, id(1).to_bytes());
    assert!(terms[0].1.iter().any(|(word, _)| word == "epsilon"));
}

const PART_1_PAYLOAD: &str = "bilbo index 1\nchunk_tokens 512\nchunk_overlap 64\nanalyzer bm25s-en/2\ntokenizer qwen/1\nrender markdown/1/heading_width=100 session/1\ncatalog 09090909090909090909090909090909\nseq 5\nfailed -\n";

fn part_1_schema() -> tantivy::schema::Schema {
    use tantivy::schema::{
        BytesOptions, FAST, INDEXED, IndexRecordOption, STRING, Schema, TextFieldIndexing,
        TextOptions,
    };
    let mut builder = Schema::builder();
    let words = TextFieldIndexing::default()
        .set_tokenizer("raw")
        .set_fieldnorms(true)
        .set_index_option(IndexRecordOption::WithFreqs);
    builder.add_text_field("text", TextOptions::default().set_indexing_options(words));
    builder.add_bytes_field("entry", BytesOptions::default().set_indexed().set_fast());
    builder.add_u64_field("generation", FAST);
    builder.add_u64_field("first", FAST);
    builder.add_u64_field("count", FAST);
    builder.add_u64_field("part", FAST);
    builder.add_u64_field("parts", FAST);
    builder.add_i64_field("date", INDEXED | FAST);
    builder.add_text_field("kind", STRING | FAST);
    builder.build()
}

#[test]
fn an_index_of_the_older_format_has_no_position() {
    use tantivy::tokenizer::{PreTokenizedString, Token};
    let scratch = Scratch::new("part-1");
    fs::create_dir_all(scratch.folder()).expect("create");
    marker(&scratch.folder());
    let schema = part_1_schema();
    let settings = tantivy::IndexSettings {
        docstore_compression: tantivy::store::Compressor::None,
        ..tantivy::IndexSettings::default()
    };
    let old = tantivy::Index::builder()
        .schema(schema.clone())
        .settings(settings)
        .create_in_dir(scratch.folder())
        .expect("an index");
    let mut writer: tantivy::IndexWriter = old.writer(15_000_000).expect("a writer");
    let field = |name: &str| schema.get_field(name).expect(name);
    for row in filled() {
        let mut document = tantivy::TantivyDocument::default();
        document.add_pre_tokenized_text(
            field("text"),
            PreTokenizedString {
                text: String::new(),
                tokens: vec![Token {
                    offset_from: 0,
                    offset_to: 0,
                    position: 0,
                    text: "word".to_owned(),
                    position_length: 1,
                }],
            },
        );
        document.add_bytes(field("entry"), &row.entry);
        document.add_u64(field("generation"), row.generation);
        document.add_u64(field("first"), row.first);
        document.add_u64(field("count"), row.count);
        document.add_u64(field("part"), row.part);
        document.add_u64(field("parts"), row.parts);
        document.add_i64(field("date"), row.date);
        document.add_text(field("kind"), &row.kind);
        writer.add_document(document).expect("add");
    }
    let mut prepared = writer.prepare_commit().expect("prepare");
    prepared.set_payload(PART_1_PAYLOAD);
    prepared.commit().expect("commit");
    drop(writer);
    drop(old);

    let before = snapshot(&scratch.0);
    let mut index = scratch.index();
    assert_eq!(index.position().expect("a position"), None);
    assert_eq!(committed(&scratch).position().expect("a position"), None);
    assert_eq!(index.extents(id(2)).expect("extents"), []);
    assert!(matches!(
        committed(&scratch).totals().expect_err("no index"),
        IndexError::Failed { .. }
    ));
    assert_eq!(snapshot(&scratch.0), before, "looking changed a file");
    let doc = Doc {
        entry: id(1),
        generation: Generation(1),
        kind: "session",
        date: UNIX_EPOCH,
        chunk: &chunk(4, 1, None, "grown"),
    };
    assert!(matches!(
        index.add(&doc).expect_err("not continued"),
        IndexError::Failed { .. }
    ));
    index.clear().expect("clear");
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    assert_eq!(index.position().expect("a position"), Some(position(5)));
    assert_eq!(index.documents().expect("documents"), filled());

    let own = Scratch::new("part-1-own");
    own.fresh().commit(&position(1)).expect("commit");
    let made = tantivy::Index::open_in_dir(own.folder())
        .expect("the adapter's index")
        .schema();
    assert_ne!(made, schema);
    for (_, entry) in schema.fields() {
        let found = made.get_field(entry.name()).expect("the field");
        assert_eq!(made.get_field_entry(found), entry, "{}", entry.name());
    }
    assert_eq!(made.fields().count(), schema.fields().count() + 2);
    for name in ["length", "digest"] {
        assert!(made.get_field(name).is_ok(), "{name}");
    }
}

#[test]
fn an_index_of_this_schema_with_a_format_1_payload_has_no_position() {
    let scratch = Scratch::new("format-1");
    built(&scratch.folder());
    assert_eq!(
        committed(&scratch).position().expect("a position"),
        Some(position(5))
    );
    let raw = tantivy::Index::open_in_dir(scratch.folder()).expect("the index");
    let mut writer: tantivy::IndexWriter = raw.writer(15_000_000).expect("a writer");
    let mut prepared = writer.prepare_commit().expect("prepare");
    prepared.set_payload(PART_1_PAYLOAD);
    prepared.commit().expect("commit");
    drop(writer);
    drop(raw);
    assert_eq!(committed(&scratch).position().expect("a position"), None);
    let mut index = scratch.index();
    assert_eq!(index.position().expect("a position"), None);
    index.clear().expect("clear");
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    assert_eq!(index.position().expect("a position"), Some(position(5)));
}

#[test]
fn the_segment_check_of_a_position_costs_little() {
    let scratch = Scratch::new("cost");
    let mut index = scratch.fresh();
    let text = "word another third fourth fifth";
    for commit in 0..3u64 {
        for n in 0..2000u64 {
            index
                .add(&Doc {
                    entry: id(1),
                    generation: Generation(1),
                    kind: "session",
                    date: UNIX_EPOCH,
                    chunk: &chunk(commit * 2000 + n, 1, None, text),
                })
                .expect("add");
        }
        index.commit(&position(commit)).expect("commit");
    }
    drop(index);
    let segments = names(&scratch.folder())
        .iter()
        .filter(|name| name.ends_with(".term"))
        .count();
    let mut reader = scratch.index();
    reader.position().expect("warm the cache");
    let runs = 50;
    let started = std::time::Instant::now();
    for _ in 0..runs {
        assert!(reader.position().expect("a position").is_some());
    }
    let each = started.elapsed() / runs;
    say(&format!(
        "position() on {segments} segments of 2000 documents: {each:?} each"
    ));
    assert!(each < Duration::from_millis(100), "{each:?}");
}

#[expect(
    clippy::disallowed_macros,
    reason = "a skipped test and a measurement say so on stderr, for whoever runs with --nocapture"
)]
fn say(text: &str) {
    eprintln!("{text}");
}

#[test]
fn a_body_damaged_under_a_good_footer_that_panics_tantivy_is_a_broken_index() {
    let reference = Scratch::new("panic-reference");
    let mut wanted = reference.fresh();
    fill(&mut wanted);
    wanted.commit(&position(5)).expect("commit");
    let mut broken = Vec::new();
    for at in 385..435usize {
        let scratch = Scratch::new("panic");
        built(&scratch.folder());
        let file = component(&scratch.folder(), ".fast");
        let mut bytes = fs::read(&file).expect("read");
        bytes[at] ^= 0x55;
        fs::write(&file, bytes).expect("write");
        let before = snapshot(&scratch.0);
        let mut index = scratch.index();
        let Ok(None) = index.position() else {
            continue;
        };
        broken.push(at);
        assert_eq!(index.extents(id(1)).expect("extents"), [], "offset {at}");
        assert_eq!(snapshot(&scratch.0), before, "offset {at}");
        index.clear().expect("clear");
        fill(&mut index);
        index.commit(&position(5)).expect("commit");
        assert_eq!(index.position().expect("a position"), Some(position(5)));
        assert_eq!(
            index.terms().expect("terms"),
            wanted.terms().expect("terms")
        );
        let mut fresh = scratch.index();
        assert_eq!(fresh.position().expect("a position"), Some(position(5)));
        fresh.clear().expect("clear");
        fill(&mut fresh);
        fresh.commit(&position(5)).expect("commit");
        assert_eq!(
            fresh.terms().expect("terms"),
            wanted.terms().expect("terms")
        );
    }
    say(&format!("offsets of .fast read as broken: {broken:?}"));
    assert!(!broken.is_empty());
}

#[test]
fn a_column_damaged_under_a_good_footer_that_panics_when_read_is_a_failed_read() {
    let reference = Scratch::new("column-reference");
    let mut wanted = reference.fresh();
    fill(&mut wanted);
    wanted.commit(&position(5)).expect("commit");
    let mut failed = Vec::new();
    for at in 60..110usize {
        let scratch = Scratch::new("column");
        built(&scratch.folder());
        let file = component(&scratch.folder(), ".fast");
        let mut bytes = fs::read(&file).expect("read");
        bytes[at] ^= 0x55;
        fs::write(&file, bytes).expect("write");
        let before = snapshot(&scratch.0);
        let mut index = scratch.index();
        let Ok(Some(_)) = index.position() else {
            continue;
        };
        let Err(error) = index.extents(id(1)) else {
            continue;
        };
        assert!(matches!(error, IndexError::Failed { .. }), "offset {at}");
        failed.push((at, error.to_string()));
        assert_eq!(snapshot(&scratch.0), before, "offset {at}");
        index.clear().expect("clear");
        fill(&mut index);
        index.commit(&position(5)).expect("commit");
        assert_eq!(index.position().expect("a position"), Some(position(5)));
        assert_eq!(
            index.terms().expect("terms"),
            wanted.terms().expect("terms")
        );
        let mut fresh = scratch.index();
        fresh.clear().expect("clear");
        fill(&mut fresh);
        fresh.commit(&position(5)).expect("commit");
        assert_eq!(
            fresh.terms().expect("terms"),
            wanted.terms().expect("terms")
        );
    }
    say(&format!("offsets of .fast that fail extents: {failed:?}"));
    assert!(
        failed
            .iter()
            .any(|(_, text)| text.contains("its content could not be read")),
        "no offset made tantivy panic while the columns were read"
    );
}

fn term_of(word: &str) -> String {
    analyze::terms(word).remove(0)
}

fn docs_of(postings: &[bilbo_search::lookup::Posting]) -> Vec<u32> {
    postings.iter().map(|posting| posting.doc).collect()
}

#[test]
fn a_documents_row_carries_its_length_and_digest() {
    let scratch = Scratch::new("row-length-digest");
    built(&scratch.folder());
    let rows = committed(&scratch).documents().expect("documents");
    assert_eq!(rows, filled());
    assert_eq!(rows[0].length, 3);
    assert_eq!(rows[0].digest, chunk::digest(RUNNER));
}

#[test]
fn what_was_added_comes_back_as_postings_lengths_kinds_and_rows() {
    let scratch = Scratch::new("lookup");
    built(&scratch.folder());
    let reader = committed(&scratch);
    let garden = term_of("garden");
    let postings = reader.postings(&garden).expect("postings");
    let found = reader.found(&docs_of(&postings)).expect("found");
    let seen: Vec<(u64, u64, u32, u32)> = postings
        .iter()
        .zip(&found)
        .map(|(posting, found)| {
            (
                found.extent.records.first,
                found.extent.part.map_or(0, |(part, _)| u64::from(part)),
                posting.freq,
                posting.length,
            )
        })
        .collect();
    assert_eq!(seen, [(1, 0, 1, 3), (4, 0, 2, 3), (7, 0, 1, 3)]);
    assert!(postings.windows(2).all(|pair| pair[0].doc < pair[1].doc));
    assert_eq!(
        found.iter().map(|found| found.digest).collect::<Vec<_>>(),
        [RUNNER, GARDENS, ZEBRA].map(chunk::digest)
    );
    assert_eq!(reader.postings("nowhere").expect("postings"), []);

    let sessions = reader.holding(&Filter::Kind("session")).expect("holding");
    let markdown = reader.holding(&Filter::Kind("markdown")).expect("holding");
    assert_eq!(sessions.len(), 3);
    assert_eq!(markdown.len(), 1);
    assert!(
        reader
            .holding(&Filter::Kind("none"))
            .expect("holding")
            .is_empty()
    );
    let all = reader.totals().expect("totals");
    assert_eq!(
        (all.documents, all.entries, all.length, u64::from(all.bound)),
        (4, 2, 3 + 3 + 2 + 3, 4)
    );

    let mut asked: Vec<u32> = sessions.iter().chain(&markdown).copied().collect();
    asked.reverse();
    let rows = reader.found(&asked).expect("found");
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].kind, "markdown");
    assert_eq!(rows[0].entry, id(2));
    assert_eq!(rows[0].extent.records, RecordRange { first: 7, count: 2 });
    assert_eq!(rows[0].extent.date, 200_000);
    assert_eq!(rows[3].extent.records, RecordRange { first: 1, count: 3 });
    assert_eq!(rows[3].extent.generation, Generation(2));
}

#[test]
fn a_documents_length_is_its_exact_number_of_terms() {
    let scratch = Scratch::new("lengths");
    let mut index = scratch.fresh();
    let lengths = [41usize, 1000, 5000];
    for (n, length) in lengths.iter().enumerate() {
        let text = "alpha ".repeat(*length);
        assert_eq!(analyze::terms(&text).len(), *length);
        add(
            &mut index,
            n as u8 + 1,
            "session",
            100,
            &chunk(1, 1, None, &text),
        );
    }
    add(
        &mut index,
        9,
        "session",
        100,
        &chunk(1, 1, None, "the and of"),
    );
    index.commit(&position(5)).expect("commit");
    let reader = committed(&scratch);
    let postings = reader.postings(&term_of("alpha")).expect("postings");
    assert_eq!(
        postings
            .iter()
            .map(|posting| (posting.freq as usize, posting.length as usize))
            .collect::<Vec<_>>(),
        lengths.map(|length| (length, length))
    );
    let rows = reader.documents().expect("documents");
    assert_eq!(
        rows.iter().map(|row| row.length).collect::<Vec<_>>(),
        [41, 1000, 5000, 0]
    );
    assert_eq!(reader.totals().expect("totals").length, 6041);
}

#[test]
fn a_replaced_document_is_in_no_posting_and_no_total() {
    let scratch = Scratch::new("replaced-document");
    let mut index = scratch.fresh();
    for entry in 1..=10 {
        add(
            &mut index,
            entry,
            "session",
            100,
            &chunk(1, 1, None, "zebra zebra"),
        );
    }
    index.commit(&position(1)).expect("commit");
    index.delete(id(1), 1).expect("delete");
    add(&mut index, 1, "markdown", 100, &chunk(1, 1, None, "yak"));
    index.commit(&position(2)).expect("commit");
    assert_eq!(
        names(&scratch.folder())
            .iter()
            .filter(|name| name.ends_with(".term"))
            .count(),
        2,
        "no merge has happened"
    );
    let reader = committed(&scratch);
    let zebra = reader.postings(&term_of("zebra")).expect("postings");
    assert_eq!(docs_of(&zebra), (1..=9).collect::<Vec<u32>>());
    assert!(zebra.iter().all(|posting| posting.freq == 2));
    assert_eq!(docs_of(&reader.postings("yak").expect("postings")), [10]);
    assert_eq!(
        reader.holding(&Filter::Kind("session")).expect("holding"),
        (1..=9).collect::<Vec<u32>>()
    );
    let totals = reader.totals().expect("totals");
    assert_eq!(
        (
            totals.documents,
            totals.entries,
            totals.length,
            totals.bound
        ),
        (10, 10, 19, 11)
    );
}

#[test]
fn the_entries_counted_are_those_with_a_live_document() {
    let scratch = Scratch::new("entries");
    let mut index = scratch.fresh();
    add(&mut index, 1, "session", 100, &chunk(1, 1, None, "one"));
    add(&mut index, 1, "session", 100, &chunk(2, 1, None, "two"));
    add(&mut index, 2, "session", 100, &chunk(1, 1, None, "three"));
    add(&mut index, 3, "session", 100, &chunk(1, 1, None, "four"));
    add(&mut index, 4, "session", 100, &chunk(1, 1, None, "five"));
    for entry in 10..=40 {
        add(
            &mut index,
            entry,
            "session",
            100,
            &chunk(1, 1, None, "filler"),
        );
    }
    index.commit(&position(1)).expect("commit");
    assert_eq!(committed(&scratch).totals().expect("totals").entries, 35);
    index.delete(id(3), 1).expect("delete");
    index.delete(id(1), 2).expect("delete");
    add(&mut index, 2, "session", 100, &chunk(2, 1, None, "six"));
    index.commit(&position(2)).expect("commit");
    let unmerged = |scratch: &Scratch| {
        let files = names(&scratch.folder());
        (
            files.iter().filter(|name| name.ends_with(".term")).count(),
            files.iter().any(|name| name.ends_with(".del")),
        )
    };
    assert_eq!(
        unmerged(&scratch),
        (2, true),
        "deletes in a segment still there"
    );
    let totals = committed(&scratch).totals().expect("totals");
    assert_eq!(
        (totals.documents, totals.entries),
        (35, 34),
        "entry 3 has no live document; entry 2 lies in two segments and counts once; entry 1 keeps its first"
    );
    index.delete(id(1), 1).expect("delete");
    index.delete(id(4), 1).expect("delete");
    index.commit(&position(3)).expect("commit");
    assert!(unmerged(&scratch).1);
    let totals = committed(&scratch).totals().expect("totals");
    assert_eq!((totals.documents, totals.entries), (33, 32));
}

#[test]
fn the_same_documents_rank_the_same_in_one_segment_and_in_many() {
    let texts: Vec<String> = (0..14)
        .map(|n| {
            format!(
                "garden {} tools {}",
                "zebra ".repeat(n % 4),
                "shed ".repeat(n % 3)
            )
        })
        .collect();
    let one = Scratch::new("one-segment");
    let mut single = one.fresh();
    let many = Scratch::new("seven-segments");
    let mut several = many.fresh();
    for (n, text) in texts.iter().enumerate() {
        let at = n as u64 + 1;
        add(
            &mut single,
            n as u8 + 1,
            "session",
            100,
            &chunk(at, 1, None, text),
        );
        add(
            &mut several,
            n as u8 + 1,
            "session",
            100,
            &chunk(at, 1, None, text),
        );
        if n % 2 == 1 {
            several.commit(&position(at)).expect("commit");
        }
    }
    single.commit(&position(14)).expect("commit");
    let terms = |scratch: &Scratch| {
        names(&scratch.folder())
            .iter()
            .filter(|name| name.ends_with(".term"))
            .count()
    };
    assert_eq!((terms(&one), terms(&many)), (1, 7));
    assert_eq!(
        committed(&one).documents().expect("documents"),
        committed(&many).documents().expect("documents")
    );
    assert_eq!(
        committed(&one).totals().expect("totals"),
        committed(&many).totals().expect("totals")
    );
    for query in ["garden", "zebra", "zebra shed", "tools garden shed", "none"] {
        for limit in [1, 3, 20] {
            assert_eq!(
                rank(&one.folder(), query, limit),
                rank(&many.folder(), query, limit),
                "{query} {limit}"
            );
        }
    }
}

type Stamp = Vec<(String, u64, std::time::SystemTime)>;

fn stamps(folder: &Path) -> Stamp {
    let mut found: Stamp = fs::read_dir(folder)
        .expect("list")
        .map(|entry| {
            let entry = entry.expect("an entry");
            let meta = entry.metadata().expect("metadata");
            (
                entry.file_name().into_string().expect("a name"),
                meta.len(),
                meta.modified().expect("a time"),
            )
        })
        .collect();
    found.push((
        String::new(),
        0,
        fs::metadata(folder)
            .expect("the folder")
            .modified()
            .expect("a time"),
    ));
    found.sort();
    found
}

fn read_everything(folder: &Path) {
    let reader = Committed::at(folder);
    assert!(reader.position().expect("position").is_some());
    let totals = reader.totals().expect("totals");
    reader.postings(&term_of("garden")).expect("postings");
    reader.holding(&Filter::Kind("session")).expect("holding");
    let all: Vec<u32> = (0..totals.bound).collect();
    reader.found(&all).expect("found");
    reader.documents().expect("documents");
    reader.terms().expect("terms");
}

#[test]
fn reading_creates_and_changes_no_file() {
    let scratch = Scratch::new("reading");
    built(&scratch.folder());
    for lock in [".tantivy-meta.lock", ".tantivy-writer.lock"] {
        let _ = fs::remove_file(scratch.folder().join(lock));
    }
    let before = stamps(&scratch.folder());
    std::thread::sleep(Duration::from_millis(20));
    read_everything(&scratch.folder());
    assert_eq!(stamps(&scratch.folder()), before);
    assert!(!scratch.folder().join(".tantivy-meta.lock").exists());

    let permissions = fs::metadata(scratch.folder())
        .expect("the folder")
        .permissions();
    let mut closed = permissions.clone();
    closed.set_mode(0o500);
    fs::set_permissions(scratch.folder(), closed).expect("close the folder");
    let outcome = std::panic::catch_unwind(|| read_everything(&scratch.folder()));
    fs::set_permissions(scratch.folder(), permissions).expect("open the folder");
    assert!(outcome.is_ok(), "a read-only folder is read");
    assert_eq!(stamps(&scratch.folder()), before);
}

fn nothing_to_read(reader: &Committed, what: &str) {
    assert_eq!(reader.position().expect(what), None, "{what}");
    let errors = [
        reader.totals().map(|_| ()),
        reader.postings("garden").map(|_| ()),
        reader.holding(&Filter::Kind("session")).map(|_| ()),
        reader.found(&[0]).map(|_| ()),
        reader.documents().map(|_| ()),
        reader.terms().map(|_| ()),
    ];
    for error in errors {
        let error = error.expect_err(what);
        assert!(
            matches!(error, IndexError::Failed { .. }),
            "{what}: {error}"
        );
        assert!(
            error.to_string().contains("it holds no index"),
            "{what}: {error}"
        );
    }
}

#[test]
fn a_missing_an_empty_or_a_broken_folder_has_no_position_and_every_other_call_fails() {
    for (what, leave) in unusable() {
        let scratch = Scratch::new("no-index");
        leave(&scratch.folder());
        let before = snapshot(&scratch.0);
        nothing_to_read(&committed(&scratch), what);
        assert_eq!(snapshot(&scratch.0), before, "{what}");
    }
}

#[test]
fn a_foreign_folder_is_an_error_from_every_call_and_is_left_as_it_was() {
    type Foreign = (&'static str, fn(&Path));
    let foreign: [Foreign; 3] = [
        ("a file that is not tantivy's", |folder| {
            fs::write(folder.join("notes.txt"), "mine").expect("write");
        }),
        ("a stranger beside a marked index", |folder| {
            built(folder);
            fs::write(folder.join("notes.txt"), "mine").expect("write");
        }),
        ("a segment file and no marker", |folder| {
            fs::write(folder.join("7827f006c8a44482bed98f5b97d6420f.idx"), "x").expect("write");
        }),
    ];
    for (what, stray) in foreign {
        let scratch = Scratch::new("foreign-reads");
        fs::create_dir_all(scratch.folder()).expect("create");
        stray(&scratch.folder());
        refused(&scratch, what);
    }
}

#[test]
fn a_segment_file_removed_between_two_looks_is_no_usable_index_not_a_panic() {
    for extension in [".idx", ".term", ".fast", ".fieldnorm", ".store"] {
        let scratch = Scratch::new("removed");
        built(&scratch.folder());
        let file = component(&scratch.folder(), extension);
        let (reader, looks) = counted(&scratch.folder(), move |_| {
            let _ = fs::remove_file(&file);
        });
        assert_eq!(reader.position().expect(extension), None, "{extension}");
        assert_eq!(looks.get(), 2, "{extension}");
        nothing_to_read(&committed(&scratch), extension);
    }
}

fn a_tie() -> (Scratch, usize) {
    let scratch = Scratch::new("tie");
    let mut index = scratch.fresh();
    let count = 20_000usize;
    let same = chunk(1, 1, None, "same words here");
    for n in 0..count {
        let mut entry = [0u8; 16];
        entry[12..].copy_from_slice(&(n as u32).to_be_bytes());
        index
            .add(&Doc {
                entry: Id::from_bytes(entry),
                generation: Generation(1),
                kind: "session",
                date: UNIX_EPOCH,
                chunk: &same,
            })
            .expect("add");
    }
    index.commit(&position(1)).expect("commit");
    (scratch, count)
}

struct Counting<'a> {
    inner: &'a Committed,
    widest: std::cell::Cell<usize>,
}

impl Lookup for Counting<'_> {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.inner.position()
    }
    fn totals(&self) -> Result<bilbo_search::lookup::Totals, IndexError> {
        self.inner.totals()
    }
    fn postings(&self, term: &str) -> Result<Vec<bilbo_search::lookup::Posting>, IndexError> {
        self.inner.postings(term)
    }
    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.inner.holding(filter)
    }
    fn found(&self, docs: &[u32]) -> Result<Vec<bilbo_search::lookup::Found>, IndexError> {
        self.widest.set(self.widest.get().max(docs.len()));
        self.inner.found(docs)
    }
}

#[test]
fn a_tie_of_every_document_is_cut_by_the_tie_order() {
    let (scratch, count) = a_tie();
    let reader = committed(&scratch);
    let counting = Counting {
        inner: &reader,
        widest: std::cell::Cell::new(0),
    };
    let query = Query {
        text: "same words".to_owned(),
        kinds: Vec::new(),
    };
    let hits = Bm25 {
        index: &counting,
        k1: 1.2,
        b: 0.75,
        omit: Omit::default(),
        batch: 4096,
    }
    .rank(&query, 50)
    .expect("a rank");
    let wanted: Vec<u32> = (0..50).collect();
    let entries: Vec<u32> = hits
        .iter()
        .map(|hit| {
            let bytes = hit.found.entry.to_bytes();
            u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]])
        })
        .collect();
    assert_eq!(entries, wanted);
    assert_eq!(reader.totals().expect("totals").documents, count as u64);
    assert!(counting.widest.get() <= 4096, "{}", counting.widest.get());
    assert!(counting.widest.get() > 0);
}

fn template() -> Scratch {
    let scratch = Scratch::new("template");
    built(&scratch.folder());
    scratch
}

fn flipped(template: &Scratch, extension: &str, at: usize) -> Scratch {
    let scratch = Scratch::new("flipped");
    copy_files(&template.folder(), &scratch.folder());
    let file = component(&scratch.folder(), extension);
    let mut bytes = fs::read(&file).expect("read");
    bytes[at] ^= 0x55;
    fs::write(&file, bytes).expect("write");
    scratch
}

fn length_of(extension: &str, template: &Scratch) -> usize {
    fs::metadata(component(&template.folder(), extension))
        .expect("a file")
        .len() as usize
}

#[test]
fn a_damaged_length_column_is_an_index_error_from_totals_and_postings() {
    let template = template();
    let size = length_of(".fast", &template);
    let garden = term_of("garden");
    let mut both = Vec::new();
    let mut panicked = Vec::new();
    for at in 0..size {
        let scratch = flipped(&template, ".fast", at);
        let before = snapshot(&scratch.0);
        let reader = committed(&scratch);
        if !matches!(reader.position(), Ok(Some(_))) {
            continue;
        }
        let Err(error) = reader.totals() else {
            continue;
        };
        assert!(matches!(error, IndexError::Failed { .. }), "offset {at}");
        if error.to_string().contains("its content could not be read") {
            panicked.push(at);
        }
        let Err(posting_error) = reader.postings(&garden) else {
            panic!("offset {at}: totals failed on the length column and postings read it");
        };
        assert!(
            matches!(posting_error, IndexError::Failed { .. }),
            "offset {at}"
        );
        both.push(at);
        assert_eq!(snapshot(&scratch.0), before, "offset {at}");
    }
    say(&format!(
        "offsets of .fast where totals and postings fail: {both:?}; by a caught panic: {panicked:?}"
    ));
    assert!(!both.is_empty(), "no offset damaged the length column");
    assert!(
        !panicked.is_empty(),
        "no offset made tantivy panic while the length column was read"
    );
}

#[test]
fn a_damaged_postings_or_term_file_is_an_index_error_from_every_read() {
    let mut failed_lazily = 0;
    let template = template();
    for extension in [".idx", ".term"] {
        let size = length_of(extension, &template);
        for at in 0..size {
            let scratch = flipped(&template, extension, at);
            let before = snapshot(&scratch.0);
            let reader = committed(&scratch);
            let opened = matches!(reader.position(), Ok(Some(_)));
            let bound = if opened {
                reader.totals().map_or(4, |totals| totals.bound)
            } else {
                4
            };
            let all: Vec<u32> = (0..bound).collect();
            let results = [
                reader.totals().map(|_| ()),
                reader.postings(&term_of("garden")).map(|_| ()),
                reader.holding(&Filter::Kind("session")).map(|_| ()),
                reader.found(&all).map(|_| ()),
                reader.documents().map(|_| ()),
                reader.terms().map(|_| ()),
            ];
            for result in results {
                match result {
                    Ok(()) => {}
                    Err(IndexError::Failed { .. } | IndexError::Inconsistent(_)) => {
                        failed_lazily += usize::from(opened);
                    }
                    Err(error) => panic!("{extension} offset {at}: {error}"),
                }
            }
            assert_eq!(snapshot(&scratch.0), before, "{extension} offset {at}");
        }
    }
    say(&format!(
        "reads that failed after the segments opened: {failed_lazily}"
    ));
    assert!(failed_lazily > 0, "no damage reached a lazy read");
}

#[test]
fn a_column_that_cannot_be_read_is_a_failed_read_of_its_content() {
    let template = template();
    let size = length_of(".fast", &template);
    let mut failed = Vec::new();
    for at in 0..size {
        let scratch = flipped(&template, ".fast", at);
        let reader = committed(&scratch);
        if !matches!(reader.position(), Ok(Some(_))) {
            continue;
        }
        let Err(error) = reader.found(&[0, 1, 2, 3]) else {
            continue;
        };
        assert!(matches!(error, IndexError::Failed { .. }), "offset {at}");
        failed.push((at, error.to_string()));
    }
    assert!(
        failed
            .iter()
            .any(|(_, text)| text.contains("its content could not be read")),
        "no offset made a column panic through found: {failed:?}"
    );
}

fn copy_files(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create");
    for name in names(from) {
        fs::copy(from.join(&name), to.join(name)).expect("copy");
    }
}

fn remove_files(folder: &Path) {
    for name in names(folder) {
        fs::remove_file(folder.join(name)).expect("remove");
    }
}

fn counted(folder: &Path, each: impl Fn(u32) + 'static) -> (Committed, Rc<Cell<u32>>) {
    let looks = Rc::new(Cell::new(0));
    let seen = Rc::clone(&looks);
    let reader = Committed::watched(
        folder,
        Box::new(move || {
            seen.set(seen.get() + 1);
            each(seen.get());
        }),
    );
    (reader, looks)
}

fn pad_meta(scratch: &Scratch, size: u64) {
    let file = fs::OpenOptions::new()
        .append(true)
        .open(scratch.folder().join("meta.json"))
        .expect("meta.json");
    let held = file.metadata().expect("metadata").len();
    std::io::Write::write_all(
        &mut &file,
        &vec![b' '; usize::try_from(size - held).expect("a size")],
    )
    .expect("pad");
}

#[test]
fn a_meta_file_over_the_bound_is_no_usable_index() {
    const BOUND: u64 = 64 * 1024 * 1024;
    let under = Scratch::new("meta-under");
    built(&under.folder());
    pad_meta(&under, BOUND);
    assert_eq!(
        committed(&under).position().expect("a position"),
        Some(position(5))
    );
    assert_eq!(committed(&under).documents().expect("documents"), filled());

    let over = Scratch::new("meta-over");
    built(&over.folder());
    pad_meta(&over, BOUND + 1);
    let before = snapshot(&over.0);
    nothing_to_read(&committed(&over), "a meta.json over the bound");
    assert_eq!(snapshot(&over.0), before);
}

fn other_index(scratch: &Scratch) {
    let mut index = scratch.fresh();
    add(
        &mut index,
        7,
        "markdown",
        700,
        &chunk(1, 1, None, "otter river"),
    );
    add(
        &mut index,
        7,
        "markdown",
        700,
        &chunk(2, 1, None, "otter bank"),
    );
    index.commit(&position(9)).expect("commit");
}

#[test]
fn a_commit_that_replaces_the_segments_between_two_looks_is_read_at_the_second_look() {
    let second = Scratch::new("second-index");
    other_index(&second);
    let scratch = Scratch::new("replaced-between");
    built(&scratch.folder());
    let (folder, from) = (scratch.folder(), second.folder());
    let (reader, looks) = counted(&scratch.folder(), move |look| {
        if look == 1 {
            remove_files(&folder);
            copy_files(&from, &folder);
        }
    });
    assert_eq!(reader.position().expect("a position"), Some(position(9)));
    assert_eq!(reader.totals().expect("totals").documents, 2);
    let rows = reader.documents().expect("documents");
    assert_eq!(
        rows,
        [
            row(7, "markdown", 700, 1, 1, (0, 0), "otter river"),
            row(7, "markdown", 700, 2, 1, (0, 0), "otter bank"),
        ]
    );
    assert_eq!(looks.get(), 2);
}

#[test]
fn a_commit_between_the_metadata_and_the_segments_leaves_one_commit_read() {
    let scratch = Scratch::new("commit-between");
    let mut index = scratch.fresh();
    fill(&mut index);
    index.commit(&position(5)).expect("commit");
    let kept = Scratch::new("after-first");
    copy_files(&scratch.folder(), &kept.folder());
    add(
        &mut index,
        3,
        "session",
        300,
        &chunk(1, 1, None, "one more"),
    );
    index.commit(&position(6)).expect("commit");
    drop(index);
    assert_eq!(
        names(&scratch.folder())
            .iter()
            .filter(|name| name.ends_with(".term"))
            .count(),
        2,
        "no merge: the second commit keeps the first's segment"
    );
    let next = Scratch::new("after-second");
    copy_files(&scratch.folder(), &next.folder());
    remove_files(&scratch.folder());
    copy_files(&kept.folder(), &scratch.folder());
    let (folder, from) = (scratch.folder(), next.folder());
    let (reader, looks) = counted(&scratch.folder(), move |_| {
        copy_files(&from, &folder);
    });
    assert_eq!(reader.position().expect("a position"), Some(position(5)));
    assert_eq!(reader.totals().expect("totals").documents, 4);
    assert_eq!(reader.documents().expect("documents"), filled());
    assert_eq!(looks.get(), 1);
}

#[test]
fn a_broken_index_is_judged_once() {
    let scratch = Scratch::new("judged-once");
    built(&scratch.folder());
    let file = component(&scratch.folder(), ".term");
    fs::write(&file, [0u8; 4]).expect("cut");
    let (reader, looks) = counted(&scratch.folder(), |_| {});
    assert_eq!(reader.position().expect("a position"), None);
    assert_eq!(looks.get(), 1);

    let template = template();
    let mut refused = 0;
    for at in 385..435 {
        let scratch = flipped(&template, ".fast", at);
        let (reader, looks) = counted(&scratch.folder(), |_| {});
        if matches!(reader.position(), Ok(None)) {
            refused += 1;
            assert_eq!(looks.get(), 1, "offset {at}");
        }
    }
    assert!(refused > 0, "no damage made the open of a segment fail");

    let absent = Scratch::new("judged-never");
    let (reader, looks) = counted(&absent.folder(), |_| {});
    assert_eq!(reader.position().expect("a position"), None);
    assert_eq!(looks.get(), 0);
}

#[test]
fn a_damaged_postings_or_term_file_is_an_index_error_from_extents() {
    let template = template();
    let mut failed = 0;
    for extension in [".idx", ".term"] {
        for at in 0..length_of(extension, &template) {
            let scratch = flipped(&template, extension, at);
            match scratch.index().extents(id(1)) {
                Ok(_) => {}
                Err(IndexError::Failed { .. } | IndexError::Inconsistent(_)) => failed += 1,
                Err(error) => panic!("{extension} offset {at}: {error}"),
            }
        }
    }
    assert!(failed > 0, "no damage reached a lazy read of extents");
}

fn two_documents(scratch: &Scratch) {
    let mut index = scratch.fresh();
    add(
        &mut index,
        1,
        "session",
        100,
        &chunk(1, 1, None, "zebra one"),
    );
    add(
        &mut index,
        2,
        "session",
        100,
        &chunk(1, 1, None, "zebra two"),
    );
    index.commit(&position(5)).expect("commit");
}

#[test]
fn a_segment_whose_bound_disagrees_with_its_columns_is_no_usable_index() {
    let unedited = Scratch::new("bound-unedited");
    two_documents(&unedited);
    assert_eq!(committed(&unedited).totals().expect("totals").documents, 2);
    for wrong in [1, 3] {
        let scratch = Scratch::new("bound-edited");
        two_documents(&scratch);
        let meta = scratch.folder().join("meta.json");
        let text = fs::read_to_string(&meta).expect("meta.json");
        assert_eq!(text.matches("\"max_doc\": 2").count(), 1, "{text}");
        fs::write(
            &meta,
            text.replace("\"max_doc\": 2", &format!("\"max_doc\": {wrong}")),
        )
        .expect("write");
        let before = snapshot(&scratch.0);
        nothing_to_read(&committed(&scratch), &format!("max_doc {wrong}"));
        assert_eq!(snapshot(&scratch.0), before);
    }
}

#[test]
fn a_posting_outside_its_segment_is_an_index_error() {
    let long = Scratch::new("postings-of-three");
    let mut index = long.fresh();
    for entry in 1..=3 {
        add(
            &mut index,
            entry,
            "session",
            100,
            &chunk(1, 1, None, "zebra"),
        );
    }
    index.commit(&position(5)).expect("commit");
    drop(index);
    let short = Scratch::new("postings-of-two");
    two_documents(&short);
    let untouched = Scratch::new("postings-of-two-untouched");
    copy_files(&short.folder(), &untouched.folder());
    for extension in [".idx", ".term"] {
        fs::copy(
            component(&long.folder(), extension),
            component(&short.folder(), extension),
        )
        .expect("replace");
    }
    let reader = committed(&short);
    let Some(_) = reader.position().expect("a position") else {
        panic!("the folder does not open: no test reaches the per-posting check");
    };
    let error = reader
        .postings(&term_of("zebra"))
        .expect_err("a posting past the segment");
    assert!(matches!(error, IndexError::Inconsistent(_)), "{error}");
    let error = reader
        .holding(&Filter::Kind("session"))
        .expect_err("a posting past the segment");
    assert!(matches!(error, IndexError::Inconsistent(_)), "{error}");
    let errors = [
        committed(&short).totals().map(|_| ()),
        committed(&short).terms().map(|_| ()),
        short.index().extents(id(3)).map(|_| ()),
    ];
    for error in errors {
        let error = error.expect_err("a posting past the segment");
        assert!(matches!(error, IndexError::Inconsistent(_)), "{error}");
    }

    let reader = committed(&untouched);
    assert_eq!(reader.totals().expect("totals").entries, 2);
    assert_eq!(reader.terms().expect("terms").len(), 2);
    assert_eq!(untouched.index().extents(id(1)).expect("extents").len(), 1);
}
