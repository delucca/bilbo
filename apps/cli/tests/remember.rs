use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::env::Who;
use bilbo_cli::console::settings::show_follows;
use bilbo_cli::remember::disk::Disk;
use bilbo_entries::files::{Files, Node};
use bilbo_entries::follows::Follow;
use bilbo_store::entries::{Cursor, Entries, Generation, Head, Meta, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::labels::CaptureLabel;
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Options, Store};

mod sandbox;
mod scratch;
mod stores;

use sandbox::{
    ID_A, ID_B, Run, counts, moved_line, note_with, run, run_as, run_reading, text, write,
};
use scratch::{scratch_dir, vars};
use stores::{backdate, layout};

const WAIT: Duration = Duration::from_secs(10);

impl Run {
    fn counts(&self) -> &str {
        self.stdout.lines().next().unwrap_or_default()
    }

    fn totals(&self) -> &str {
        self.stdout.lines().nth(1).unwrap_or_default()
    }
}

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let dir = scratch_dir(test);
        for sub in ["home", "notes", "config"] {
            fs::create_dir_all(dir.join(sub)).expect("create scratch");
        }
        fs::write(dir.join("config/config.toml"), "").expect("an empty config file");
        Sandbox { dir }
    }

    fn notes(&self) -> PathBuf {
        self.dir.join("notes")
    }

    fn vars(&self) -> HashMap<String, OsString> {
        vars(&self.dir)
    }

    fn layout(&self) -> Layout {
        layout(&self.vars())
    }

    fn open_seg(&self) -> PathBuf {
        self.layout().state.join("open.seg")
    }

    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.notes().join(name);
        write(&path, bytes);
        path
    }

    fn remember(&self, args: &[&str]) -> Run {
        self.piped(args, &[])
    }

    fn remember_notes(&self) -> Run {
        let notes = self.notes();
        self.remember(&[notes.to_str().expect("UTF-8")])
    }

    fn segments(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(self.layout().root)
            .expect("list the store")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "seg"))
            .collect();
        found.sort();
        found
    }

    fn entries(&self) -> Vec<(Meta, Head)> {
        let store = Store::open_read_only(&self.layout(), Options::default(), &mut |_| {})
            .expect("open read-only");
        let (changes, _) = store.changes(&Cursor::start()).expect("changes");
        let ids: BTreeSet<Id> = changes
            .iter()
            .map(bilbo_store::entries::Change::id)
            .collect();
        let mut found = Vec::new();
        for id in ids {
            let meta = store.meta(id).expect("meta").expect("a meta");
            let label = meta.labels.capture.clone().expect("a capture label");
            let head = store
                .by_capture(&label)
                .expect("by_capture")
                .expect("a head");
            found.push((meta, head));
        }
        found
    }

    fn entry(&self, file: &Path) -> (Meta, Head) {
        let label = CaptureLabel::file(file).expect("a capture label");
        self.entries()
            .into_iter()
            .find(|(meta, _)| meta.labels.capture.as_ref() == Some(&label))
            .unwrap_or_else(|| panic!("no entry for {}", file.display()))
    }

    fn generation(&self, id: Id, generation: Generation, records: u64) -> Vec<u8> {
        let store = Store::open_read_only(&self.layout(), Options::default(), &mut |_| {})
            .expect("open read-only");
        store
            .read(
                id,
                generation,
                RecordRange {
                    first: 1,
                    count: records,
                },
            )
            .expect("read")
            .concat()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn remembering_a_folder_keeps_each_md_file_once() {
    let sandbox = Sandbox::new("a_folder_once");
    sandbox.write("a.md", "one\ntwo\n");
    sandbox.write("b.md", "three\n");
    sandbox.write("sub/c.md", "four\n");
    sandbox.write("x.txt", "not markdown\n");

    let first = sandbox.remember_notes();
    assert_eq!(first.code, 0, "{:?}", first.stderr);
    assert_eq!(first.counts(), counts(3, 0, 0, 0, 0));
    assert!(
        first
            .totals()
            .starts_with("records 4, raw bytes 19, stored bytes "),
        "{}",
        first.totals()
    );
    assert!(!first.totals().ends_with(" 0"), "{}", first.totals());
    assert_eq!(first.stderr, Vec::<String>::new());
    assert_eq!(sandbox.entries().len(), 3);

    let second = sandbox.remember_notes();
    assert_eq!(second.code, 0);
    assert_eq!(second.counts(), counts(0, 0, 0, 3, 0));
    assert_eq!(second.totals(), "records 0, raw bytes 0, stored bytes 0");
}

#[test]
fn an_unchanged_file_is_skipped_a_grown_one_appends_and_an_edited_one_starts_a_new_generation() {
    let sandbox = Sandbox::new("grow_and_edit");
    let file = sandbox.write("a.md", "one\ntwo\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    let (meta, first) = sandbox.entry(&file);
    assert_eq!(first.records, 2);

    assert_eq!(sandbox.remember_notes().counts(), counts(0, 0, 0, 1, 0));

    fs::write(&file, "one\ntwo\nthree\n").expect("grow");
    let grown = sandbox.remember_notes();
    assert_eq!(grown.counts(), counts(0, 1, 0, 0, 0));
    assert_eq!(grown.totals().split(',').next(), Some("records 1"));
    let (_, head) = sandbox.entry(&file);
    assert_eq!((head.generation, head.records), (first.generation, 3));

    fs::write(&file, "ONE\ntwo\nthree\n").expect("edit");
    let edited = sandbox.remember_notes();
    assert_eq!(edited.counts(), counts(0, 0, 1, 0, 0));
    let (_, head) = sandbox.entry(&file);
    assert_eq!(head.generation, Generation(first.generation.0 + 1));
    assert_eq!(
        sandbox.generation(meta.id, head.generation, head.records),
        b"ONE\ntwo\nthree\n"
    );
    assert_eq!(
        sandbox.generation(meta.id, first.generation, 3),
        b"one\ntwo\nthree\n"
    );
}

#[test]
fn a_grown_last_line_without_a_newline_starts_a_new_generation() {
    let sandbox = Sandbox::new("last_line");
    sandbox.write("a.md", "a\nb");
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    sandbox.write("a.md", "a\nbc\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(0, 0, 1, 0, 0));
}

#[test]
fn a_frontmatter_id_becomes_the_entry_id_and_a_file_without_one_gets_a_fresh_ulid() {
    let sandbox = Sandbox::new("frontmatter_id");
    let with = sandbox.write("a.md", format!("---\nid: {ID_A}\n---\n# A\n"));
    let without = sandbox.write("b.md", "# B\n");
    let bad = sandbox.write("c.md", "---\nid: 01jc5q3v7xkm4p8r2t6w9yb0de\n---\n# C\n");

    assert_eq!(sandbox.remember_notes().counts(), counts(3, 0, 0, 0, 0));
    let held = Id::parse(ID_A).expect("a valid id");
    assert_eq!(sandbox.entry(&with).0.id, held);
    let fresh = sandbox.entry(&without).0.id;
    let lowercase = sandbox.entry(&bad).0.id;
    assert_ne!(fresh, held);
    assert_ne!(lowercase, held);
    assert_ne!(fresh, lowercase);
}

#[test]
fn an_id_another_file_holds_is_refused_and_the_run_goes_on() {
    let note = format!("---\nid: {ID_A}\n---\n# A\n");

    let one_run = Sandbox::new("clash_one_run");
    let first = one_run.write("a.md", &note);
    let second = one_run.write("b.md", &note);
    one_run.write("c.md", format!("---\nid: {ID_B}\n---\n# C\n"));
    let run = one_run.remember_notes();
    assert_eq!(run.code, 1);
    assert_eq!(run.counts(), counts(2, 0, 0, 0, 1));
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: refused {}: its id {ID_A} is held by file:{}",
            text(&second),
            text(&first)
        )]
    );

    let two_runs = Sandbox::new("clash_two_runs");
    let first = two_runs.write("a.md", &note);
    assert_eq!(two_runs.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    let copy = two_runs.write("copy.md", &note);
    let run = two_runs.remember_notes();
    assert_eq!(run.code, 1);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 1));
    assert!(
        run.stderr[0].starts_with(&format!("bilbo: refused {}: its id ", text(&copy))),
        "{:?}",
        run.stderr
    );
    assert!(run.stderr[0].ends_with(&format!("is held by file:{}", text(&first))));
}

fn stored_bytes(run: &Run) -> u64 {
    run.totals()
        .rsplit(' ')
        .next()
        .and_then(|number| number.parse().ok())
        .expect("a stored bytes count")
}

#[test]
fn a_renamed_file_with_an_id_is_the_same_entry_under_its_new_label() {
    let sandbox = Sandbox::new("renamed");
    let old = sandbox.write("decision-x.md", note_with(ID_A, "A"));
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    let new = sandbox.notes().join("report-x.md");
    fs::rename(&old, &new).expect("rename");

    let run = sandbox.remember(&[text(&new)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 0));
    assert!(
        run.totals()
            .starts_with("records 0, raw bytes 0, stored bytes ")
    );
    assert!(stored_bytes(&run) > 0);
    assert_eq!(run.stderr, [moved_line(&new, ID_A, &old)]);
    let entries = sandbox.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0.id, Id::parse(ID_A).expect("an id"));
    assert_eq!(entries[0].0.labels.capture, CaptureLabel::file(&new));

    let again = sandbox.remember_notes();
    assert_eq!(again.code, 0);
    assert_eq!(again.counts(), counts(0, 0, 0, 1, 0));
    assert_eq!(again.totals(), "records 0, raw bytes 0, stored bytes 0");
    assert_eq!(again.stderr, Vec::<String>::new());
}

#[test]
fn a_renamed_file_that_changed_keeps_its_older_text_as_the_older_generation() {
    let sandbox = Sandbox::new("renamed_changed");
    let old = sandbox.write("a.md", note_with(ID_A, "A"));
    sandbox.remember_notes();
    let new = sandbox.notes().join("b.md");
    fs::rename(&old, &new).expect("rename");
    fs::write(&new, note_with(ID_A, "B")).expect("edit");

    let run = sandbox.remember(&[text(&new)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(0, 0, 1, 0, 0));
    let (meta, head) = sandbox.entry(&new);
    assert_eq!(meta.id, Id::parse(ID_A).expect("an id"));
    assert_eq!(head.generation, Generation(2));
    assert_eq!(
        sandbox.generation(meta.id, Generation(2), 4),
        note_with(ID_A, "B").into_bytes()
    );
    assert_eq!(
        sandbox.generation(meta.id, Generation(1), 4),
        note_with(ID_A, "A").into_bytes()
    );
}

#[test]
fn a_moved_folder_moves_every_entry_with_an_id_and_a_file_without_one_is_new() {
    let sandbox = Sandbox::new("moved_folder");
    sandbox.write("a.md", note_with(ID_A, "A"));
    sandbox.write("b.md", note_with(ID_B, "B"));
    sandbox.write("c.md", "# C\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(3, 0, 0, 0, 0));
    let moved = sandbox.dir.join("moved");
    fs::rename(sandbox.notes(), &moved).expect("rename the folder");

    let run = sandbox.remember(&[text(&moved)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 2, 0));
    assert_eq!(
        run.stderr,
        [
            moved_line(&moved.join("a.md"), ID_A, &sandbox.notes().join("a.md")),
            moved_line(&moved.join("b.md"), ID_B, &sandbox.notes().join("b.md")),
        ]
    );
    assert_eq!(sandbox.entries().len(), 4);
}

#[test]
fn a_rename_and_a_new_file_under_the_old_name_are_two_entries_after_the_catalog_is_made_again() {
    let sandbox = Sandbox::new("rename_and_reuse");
    let old = sandbox.write("a.md", note_with(ID_A, "A"));
    sandbox.remember_notes();
    let new = sandbox.notes().join("b.md");
    fs::rename(&old, &new).expect("rename");
    assert_eq!(sandbox.remember_notes().code, 0);
    sandbox.write("a.md", note_with(ID_B, "B"));
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 1, 0));
    fs::remove_dir_all(sandbox.layout().cache).expect("delete the cache");

    let run = sandbox.remember_notes();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(0, 0, 0, 2, 0));
    let entries = sandbox.entries();
    assert_eq!(entries.len(), 2);
    for (id, path) in [(ID_A, &new), (ID_B, &old)] {
        let found = entries
            .iter()
            .find(|(meta, _)| meta.id == Id::parse(id).expect("an id"))
            .expect("an entry");
        assert_eq!(found.0.labels.capture, CaptureLabel::file(path));
    }
}

#[test]
fn a_renamed_note_whose_old_name_already_holds_another_note_moves_at_the_first_look() {
    let sandbox = Sandbox::new("rename_over_other");
    let old = sandbox.write("a.md", note_with(ID_A, "A"));
    sandbox.remember_notes();
    let new = sandbox.notes().join("b.md");
    fs::rename(&old, &new).expect("rename");
    sandbox.write("a.md", note_with(ID_B, "B"));

    let run = sandbox.remember_notes();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.stderr, [moved_line(&new, ID_A, &old)]);
    let second = sandbox.remember_notes();
    assert_eq!(second.code, 0, "{:?}", second.stderr);
    let entries = sandbox.entries();
    assert_eq!(entries.len(), 2);
    let (a, head) = sandbox.entry(&new);
    assert_eq!(a.id, Id::parse(ID_A).expect("an id"));
    assert_eq!(head.generation, Generation(3));
    assert_eq!(
        sandbox.generation(a.id, head.generation, 4),
        note_with(ID_A, "A").into_bytes()
    );
    assert_eq!(
        sandbox.generation(a.id, Generation(2), 4),
        note_with(ID_B, "B").into_bytes()
    );
    assert_eq!(
        sandbox.generation(a.id, Generation(1), 4),
        note_with(ID_A, "A").into_bytes()
    );
    let (b, _) = sandbox.entry(&old);
    assert_eq!(b.id, Id::parse(ID_B).expect("an id"));
}

#[test]
fn a_note_renamed_onto_the_name_of_another_entry_is_refused_on_every_run() {
    let sandbox = Sandbox::new("rename_onto_entry");
    let a = sandbox.write("a.md", note_with(ID_A, "A"));
    let b = sandbox.write("b.md", note_with(ID_B, "Bee, a longer one"));
    assert_eq!(sandbox.remember_notes().counts(), counts(2, 0, 0, 0, 0));
    fs::remove_file(&b).expect("remove");
    fs::rename(&a, &b).expect("rename onto it");
    let line = format!(
        "bilbo: refused {}: its id {ID_A} is held by file:{}, but this path was kept as entry {ID_B}",
        text(&b),
        text(&a)
    );
    let before = (sandbox.entry(&a), sandbox.entry(&b));
    for _ in 0..2 {
        let run = sandbox.remember(&[text(&b)]);
        assert_eq!(run.code, 1);
        assert!(run.counts().ends_with("refused 1"), "{}", run.counts());
        assert_eq!(run.stderr, std::slice::from_ref(&line));
        let entries = sandbox.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(sandbox.entry(&a).0.id, Id::parse(ID_A).expect("an id"));
        assert_eq!(sandbox.entry(&b).0.id, Id::parse(ID_B).expect("an id"));
        assert_eq!((sandbox.entry(&a), sandbox.entry(&b)), before);
    }

    let swapped = Sandbox::new("swapped_names");
    let x = swapped.write("x.md", note_with(ID_A, "A"));
    let y = swapped.write("y.md", note_with(ID_B, "Bee, a longer one"));
    swapped.remember_notes();
    let kept = (swapped.entry(&x), swapped.entry(&y));
    let aside = swapped.notes().join("aside.txt");
    fs::rename(&x, &aside).expect("aside");
    fs::rename(&y, &x).expect("y to x");
    fs::rename(&aside, &y).expect("x to y");
    let run = swapped.remember_notes();
    assert_eq!(run.code, 1);
    assert!(run.counts().ends_with("refused 2"), "{}", run.counts());
    assert_eq!(run.stderr.len(), 2, "{:?}", run.stderr);
    assert_eq!((swapped.entry(&x), swapped.entry(&y)), kept);
}

#[test]
fn a_move_is_not_said_when_the_runs_store_fails_at_the_close() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("move_failed_close");
    let old = sandbox.write("a.md", note_with(ID_A, "A"));
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    let new = sandbox.notes().join("b.md");
    fs::rename(&old, &new).expect("rename");
    backdate(&sandbox.layout().state, 61);
    let root = sandbox.layout().root;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).expect("lock the root");
    let run = sandbox.remember(&[text(&new)]);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("unlock the root");

    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 0));
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(
        run.stderr[0].starts_with("bilbo: cannot "),
        "{:?}",
        run.stderr
    );

    let next = sandbox.remember(&[text(&new)]);
    assert_eq!(next.code, 0, "{:?}", next.stderr);
    assert_eq!(next.counts(), counts(0, 0, 0, 1, 0));
    assert!(
        next.stderr
            .iter()
            .chain(&next.notices)
            .all(|line| !line.starts_with("bilbo: moved")),
        "{:?} {:?}",
        next.stderr,
        next.notices
    );
    let entries = sandbox.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0.id, Id::parse(ID_A).expect("an id"));
    assert_eq!(entries[0].0.labels.capture, CaptureLabel::file(&new));

    let other = Sandbox::new("move_failed_follow");
    let old = other.write("a.md", note_with(ID_A, "A"));
    assert_eq!(other.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    let new = other.notes().join("b.md");
    fs::rename(&old, &new).expect("rename");
    write(
        &other.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = \"{}\"\nkind = \"pdf\"\n",
            text(&other.notes())
        )
        .as_bytes(),
    );
    fs::write(other.layout().state.join("store.again"), b"1").expect("the flag another bilbo set");
    let run = other.remember(&[text(&new)]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 0));
    assert_eq!(
        run.stderr,
        [
            moved_line(&new, ID_A, &old),
            format!(
                "bilbo: follow {}: unknown kind 'pdf'; kinds: markdown, session",
                text(&other.notes())
            ),
            "bilbo: ran the follows again for another bilbo: new 0, appended 0, new generation 0, unchanged 0, refused 0".to_owned(),
        ]
    );
}

#[test]
fn a_pass_served_for_another_bilbo_says_what_it_moved_and_withdrew() {
    let sandbox = Sandbox::new("served_moved_withdrew");
    let a = sandbox.write("a.md", note_with(ID_A, "A"));
    let b = sandbox.write("b.md", "b\n");
    sandbox.write("c.md", "c\n");
    write(
        &sandbox.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
            text(&sandbox.notes())
        )
        .as_bytes(),
    );
    assert_eq!(sandbox.remember_notes().counts(), counts(3, 0, 0, 0, 0));
    let id = sandbox.entry(&b).0.id;
    let renamed = sandbox.notes().join("a2.md");
    fs::rename(&a, &renamed).expect("rename");
    fs::remove_file(&b).expect("remove");
    let other = sandbox.dir.join("other/x.md");
    write(&other, b"x\n");
    fs::write(sandbox.layout().state.join("store.again"), b"1").expect("the flag");

    let run = sandbox.remember(&[text(&other)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [
            moved_line(&renamed, ID_A, &a),
            format!(
                "bilbo: withdrew {}: its file is gone; bilbo read {id}#g1 prints the outline of what it held",
                text(&b)
            ),
            "bilbo: ran the follows again for another bilbo: new 0, appended 0, new generation 1, unchanged 2, refused 0".to_owned(),
        ]
    );
}

const BARE: &str = "bilbo: missing <file|folder>, - or --edit; bilbo refresh keeps every follow";

impl Sandbox {
    fn piped(&self, args: &[&str], input: &[u8]) -> Run {
        run(
            self.vars(),
            &[&["remember"][..], args].concat(),
            input,
            None,
        )
    }

    fn no_store_folders(&self) {
        for folder in ["store", "state", "cache"] {
            assert!(!self.dir.join(folder).exists(), "{folder}");
        }
    }
}

fn kept_id(run: &Run) -> String {
    run.stderr
        .iter()
        .find_map(|line| line.strip_prefix("bilbo: kept the note as "))
        .expect("the kept line")
        .to_owned()
}

fn hit_headers(stdout: &str) -> Vec<Vec<&str>> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("=== "))
        .map(|header| header.split('\t').collect())
        .collect()
}

#[test]
fn a_note_from_standard_input_is_a_new_entry_that_recall_finds_at_once() {
    let sandbox = Sandbox::new("piped_note");
    let run = sandbox.piped(&["-"], b"The beet seeds are in the drawer.\n");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
    assert_eq!(run.stdout.lines().count(), 2, "{}", run.stdout);
    let numbers: Vec<u64> = run
        .totals()
        .strip_prefix("records 1, raw bytes ")
        .and_then(|rest| rest.split_once(", stored bytes "))
        .map(|(raw, stored)| [raw, stored])
        .into_iter()
        .flatten()
        .map(|number| number.parse().expect("a whole number"))
        .collect();
    assert_eq!(numbers.len(), 2, "{}", run.totals());
    assert!(numbers.iter().all(|number| *number > 0), "{}", run.totals());
    let id = kept_id(&run);
    let entries = sandbox.entries();
    assert_eq!(entries.len(), 1);
    let (meta, head) = &entries[0];
    assert_eq!(meta.id.to_string(), id);
    assert_eq!(meta.kind, "note");
    assert_eq!(
        meta.labels.capture.as_ref().map(ToString::to_string),
        Some(format!("note:{id}"))
    );
    assert_eq!(head.records, 1);

    let found = run_in(&sandbox, &["recall", "--", "beet"]);
    assert_eq!(found.code, 0, "{:?}", found.stderr);
    let headers = hit_headers(&found.stdout);
    assert_eq!(headers.len(), 1, "{}", found.stdout);
    assert_eq!(headers[0][0], format!("{}#r1", &id[id.len() - 10..]));
    assert_eq!(headers[0][2], "note", "{headers:?}");
}

fn run_in(sandbox: &Sandbox, args: &[&str]) -> Run {
    run(sandbox.vars(), args, &[], None)
}

#[test]
fn the_same_text_twice_is_two_notes() {
    let sandbox = Sandbox::new("piped_twice");
    let first = sandbox.piped(&["-"], b"Plant the beets in March.\n");
    let second = sandbox.piped(&["-"], b"Plant the beets in March.\n");
    assert_eq!(first.counts(), counts(1, 0, 0, 0, 0));
    assert_eq!(second.counts(), counts(1, 0, 0, 0, 0));
    assert_ne!(kept_id(&first), kept_id(&second));
    assert_eq!(sandbox.entries().len(), 2);
}

#[test]
fn an_empty_note_or_one_that_is_not_text_keeps_nothing_and_makes_no_store() {
    for input in [&b""[..], b" \n\t \n"] {
        let sandbox = Sandbox::new("piped_empty");
        let run = sandbox.piped(&["-"], input);
        assert_eq!(run.code, 1, "{:?}", run.stderr);
        assert_eq!(run.counts(), counts(0, 0, 0, 0, 0));
        assert_eq!(run.totals(), "records 0, raw bytes 0, stored bytes 0");
        assert_eq!(run.stderr, ["bilbo: nothing to keep: the note is empty"]);
        sandbox.no_store_folders();
    }
    let sandbox = Sandbox::new("piped_not_utf8");
    let run = sandbox.piped(&["-"], &[0xff, 0xfe, b'\n']);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, ["bilbo: refused the note: it is not UTF-8"]);
}

struct Asked(Rc<Cell<bool>>);

impl io::Read for Asked {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        self.0.set(true);
        Err(io::Error::other("asked"))
    }
}

struct Breaking(usize);

impl io::Read for Breaking {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let bytes = b"The beet";
        if self.0 == bytes.len() {
            return Err(io::Error::other("the pipe broke"));
        }
        let count = buffer.len().min(bytes.len() - self.0);
        buffer[..count].copy_from_slice(&bytes[self.0..self.0 + count]);
        self.0 += count;
        Ok(count)
    }
}

#[test]
fn a_read_of_standard_input_that_fails_keeps_nothing() {
    let sandbox = Sandbox::new("piped_broken");
    let run = run_reading(
        sandbox.vars(),
        &["remember", "-"],
        Box::new(Breaking(0)),
        None,
    );
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        ["bilbo: cannot read standard input: the pipe broke"]
    );
    sandbox.no_store_folders();
}

#[test]
fn a_note_kept_after_a_wait_for_the_store_is_dated_from_the_keep() {
    fn seconds() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs()
    }

    let sandbox = Sandbox::new("piped_dated");
    let layout = sandbox.layout();
    fs::create_dir_all(&layout.state).expect("state folder");
    let lock = fs::File::create(layout.state.join("store.lock")).expect("lock file");
    lock.lock().expect("take the lock");
    let (sender, receiver) = mpsc::channel();
    let vars = sandbox.vars();
    let runner = thread::spawn(move || {
        run(
            vars,
            &["remember", "-"],
            b"The beet seeds are in the drawer.\n",
            Some(sender),
        )
    });
    let notice = receiver.recv_timeout(WAIT).expect("a waiting notice");
    assert_eq!(
        notice,
        "bilbo: waiting for another bilbo to finish writing the store"
    );
    let first = seconds();
    while seconds() <= first {
        thread::sleep(Duration::from_millis(20));
    }
    let second = seconds();
    lock.unlock().expect("release the lock");
    let finished = runner.join().expect("the run finishes");
    assert_eq!(finished.code, 0, "{:?}", finished.stderr);
    let id = kept_id(&finished);
    let records = sandbox.generation(Id::parse(&id).expect("an id"), Generation(1), 1);
    let record = String::from_utf8(records).expect("UTF-8");
    let at: u64 = record
        .split_once("\"at\":")
        .map(|(_, rest)| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|number| number.parse().ok())
        .expect("a time field");
    assert!(
        at >= second,
        "the note is dated {at}, the keep came after {second}"
    );
}

#[test]
fn a_note_takes_tags_and_stands_with_neither_kind_nor_follow() {
    let sandbox = Sandbox::new("piped_tags");
    let run = sandbox.piped(&["-", "--tag", "garden"], b"Mulch the beds.\n");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert!(
        run.stderr.contains(&"bilbo: tagged 1 entries".to_owned()),
        "{:?}",
        run.stderr
    );
    assert!(
        run.stderr.contains(&"bilbo: tag:garden is new".to_owned()),
        "{:?}",
        run.stderr
    );
    let entries = sandbox.entries();
    assert!(
        entries[0]
            .0
            .labels
            .tags
            .carried()
            .any(|(tag, _)| tag.as_str() == "garden")
    );

    for (extra, reason) in [
        (
            ["--kind", "markdown"],
            "bilbo: --kind does not stand with a note: its kind is note",
        ),
        (
            ["--follow", ""],
            "bilbo: --follow does not stand with a note: a follow is a file or folder",
        ),
    ] {
        let fresh = Sandbox::new("piped_options");
        let args: Vec<&str> = ["-"]
            .into_iter()
            .chain(extra.into_iter().filter(|word| !word.is_empty()))
            .collect();
        let asked = Rc::new(Cell::new(false));
        let run = run_reading(
            fresh.vars(),
            &[&["remember"][..], &args].concat(),
            Box::new(Asked(Rc::clone(&asked))),
            None,
        );
        assert!(!asked.get(), "{args:?}: standard input was read");
        assert_eq!(run.code, 2, "{args:?}");
        assert_eq!(run.stdout, "");
        assert_eq!(run.stderr, with_usage(reason), "{args:?}");
        fresh.no_store_folders();
    }
}

#[test]
fn a_written_note_is_listed_apart_and_moves_no_chunk() {
    let sandbox = Sandbox::new("piped_apart");
    sandbox.write(
        "garden.md",
        "# Garden\n\nThe beet seeds are in the drawer.\n",
    );
    assert_eq!(sandbox.remember_notes().code, 0);
    let before = run_in(&sandbox, &["recall", "--", "beet"]);
    assert_eq!(before.code, 0, "{:?}", before.stderr);

    let kept = sandbox.piped(&["-"], b"Beet greens keep well in a cold frame.\n");
    assert_eq!(kept.code, 0, "{:?}", kept.stderr);
    let id = kept_id(&kept);
    let after = run_in(&sandbox, &["recall", "--", "beet"]);
    assert_eq!(after.code, 0, "{:?}", after.stderr);
    assert!(after.stdout.starts_with(&before.stdout), "{}", after.stdout);
    let rest = &after.stdout[before.stdout.len()..];
    let headers = hit_headers(rest);
    assert_eq!(headers.len(), 1, "{rest}");
    assert_eq!(headers[0][0], format!("{}#r1", &id[id.len() - 10..]));
    assert_eq!(headers[0][2], "note", "{headers:?}");
    let mut lines = rest.lines().skip(1);
    let day = lines.next().expect("a first line");
    assert!(
        day.len() == 10
            && day
                .chars()
                .enumerate()
                .all(|(at, c)| if at == 4 || at == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }),
        "{day}"
    );
    assert_eq!(lines.next(), Some("Beet greens keep well in a cold frame."));
}

#[test]
fn the_parser_speaks_before_the_config_and_the_verb_after_it() {
    let sandbox = Sandbox::new("who_speaks");
    let notes = sandbox.notes();
    let good = sandbox.dir.join("config/config.toml");
    fs::write(&good, "= not toml\n").expect("a config that does not load");
    let config_error = sandbox.remember(&[text(&notes)]).stderr;
    assert_eq!(config_error.len(), 1, "{config_error:?}");
    let follow = sandbox.remember(&["--follow"]);
    assert_eq!(
        (follow.code, follow.stderr),
        (2, with_usage("bilbo: missing <file|folder>"))
    );
    for args in [&["-", "--kind", "markdown"][..], &[], &["--tag", "garden"]] {
        let run = sandbox.remember(args);
        assert_eq!(
            (run.code, run.stderr.as_slice()),
            (2, config_error.as_slice()),
            "{args:?}"
        );
    }
    sandbox.no_store_folders();

    fs::write(&good, "").expect("a config that loads");
    let follow = sandbox.remember(&["--follow"]);
    assert_eq!(
        (follow.code, follow.stderr),
        (2, with_usage("bilbo: missing <file|folder>"))
    );
    let kind = sandbox.remember(&["-", "--kind", "markdown"]);
    assert_eq!(
        (kind.code, kind.stderr),
        (
            2,
            with_usage("bilbo: --kind does not stand with a note: its kind is note")
        )
    );
    let bare = sandbox.remember(&[]);
    assert_eq!((bare.code, bare.stderr), (2, with_usage(BARE)));
    let tag = sandbox.remember(&["--tag", "garden"]);
    assert_eq!((tag.code, tag.stderr), (2, with_usage(BARE)));
    sandbox.no_store_folders();
}

#[test]
fn a_note_moved_out_of_a_folder_that_became_a_file_moves_its_entry() {
    let sandbox = Sandbox::new("moved_out_of_a_folder_that_became_a_file");
    let old = sandbox.write("sub/n.md", note_with(ID_A, "A"));
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    let new = sandbox.notes().join("n.md");
    fs::rename(&old, &new).expect("move up");
    fs::remove_dir(sandbox.notes().join("sub")).expect("remove the folder");
    fs::write(sandbox.notes().join("sub"), "now a file\n").expect("a file under that name");

    let run = sandbox.remember(&[text(&new)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 0));
    assert_eq!(run.stderr, [moved_line(&new, ID_A, &old)]);
}

#[test]
fn a_file_that_is_not_utf8_is_refused_and_the_run_goes_on() {
    let sandbox = Sandbox::new("not_utf8");
    sandbox.write("a.md", "fine\n");
    let bad = sandbox.write("bad.md", [b'o', b'k', 0xff, 0xfe, b'\n']);
    let run = sandbox.remember_notes();
    assert_eq!(run.code, 1);
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 1));
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(
        run.stderr[0].starts_with(&format!("bilbo: refused {}: not UTF-8", text(&bad))),
        "{:?}",
        run.stderr
    );
}

#[test]
fn hidden_entries_and_symbolic_links_the_walk_finds_are_skipped() {
    let sandbox = Sandbox::new("walk_skips");
    let kept = sandbox.write("a.md", "a\n");
    sandbox.write(".hidden.md", "h\n");
    sandbox.write(".git/c.md", "c\n");
    let outside = sandbox.dir.join("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("o.md"), "o\n").expect("outside file");
    symlink(&kept, sandbox.notes().join("link.md")).expect("file link");
    symlink(&outside, sandbox.notes().join("linked")).expect("folder link");

    let run = sandbox.remember_notes();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
    assert_eq!(sandbox.entries().len(), 1);
}

#[test]
fn a_named_link_to_a_file_is_taken_and_its_capture_label_is_the_resolved_path() {
    for kind in [None, Some("markdown")] {
        let sandbox = Sandbox::new("named_file_link");
        let target = sandbox.write("a.md", "a\n");
        let link = sandbox.dir.join("link.md");
        symlink(&target, &link).expect("link");
        let mut args = vec![text(&link)];
        if let Some(kind) = kind {
            args.extend(["--kind", kind]);
        }
        let run = sandbox.remember(&args);
        assert_eq!(run.code, 0, "{kind:?}: {:?}", run.stderr);
        assert_eq!(run.counts(), counts(1, 0, 0, 0, 0), "{kind:?}");
        let entries = sandbox.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]
                .0
                .labels
                .capture
                .as_ref()
                .map(CaptureLabel::as_str),
            Some(format!("file:{}", text(&target)).as_str())
        );
        assert_eq!(sandbox.remember_notes().counts(), counts(0, 0, 0, 1, 0));
    }
}

#[test]
fn a_named_link_to_a_folder_is_walked_and_links_inside_it_are_not() {
    let sandbox = Sandbox::new("named_folder_link");
    sandbox.write("a.md", "a\n");
    let outside = sandbox.dir.join("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("o.md"), "o\n").expect("outside file");
    symlink(&outside, sandbox.notes().join("inner")).expect("inner link");
    let linked = sandbox.dir.join("linked");
    symlink(sandbox.notes(), &linked).expect("named link");

    let run = sandbox.remember(&[text(&linked)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
    for (meta, _) in sandbox.entries() {
        let label = meta.labels.capture.expect("a capture label");
        let path = label.path().expect("a file capture label");
        assert!(path.starts_with(sandbox.notes()), "{path:?}");
    }
}

#[test]
fn a_named_hidden_file_is_taken() {
    let sandbox = Sandbox::new("named_hidden");
    let draft = sandbox.write(".draft.md", "draft\n");
    let run = sandbox.remember(&[text(&draft)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
}

#[test]
fn a_target_run_withdraws_nothing_for_a_file_that_is_gone() {
    let sandbox = Sandbox::new("file_left");
    sandbox.write("a.md", "a\n");
    let gone = sandbox.write("b.md", "b\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(2, 0, 0, 0, 0));
    fs::remove_file(&gone).expect("remove");
    let run = sandbox.remember_notes();
    assert_eq!(run.code, 0);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 0));
    assert_eq!(sandbox.entries().len(), 2);
    assert_eq!(sandbox.entry(&gone).1.records, 1);
}

#[test]
fn a_torn_tail_of_open_seg_is_cut_and_the_run_goes_on() {
    let sandbox = Sandbox::new("torn_tail");
    sandbox.write("a.md", "a\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    let open_seg = sandbox.open_seg();
    let kept = fs::metadata(&open_seg).expect("open.seg").len();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&open_seg)
        .expect("open open.seg");
    file.write_all(&[0xff, 0, 0, 0]).expect("tear");
    file.write_all(&[7; 16]).expect("junk");
    drop(file);

    let run = sandbox.remember_notes();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 0));
    assert_eq!(
        run.notices,
        [format!(
            "bilbo: cut 20 bytes of a torn frame at byte {kept} of {}",
            text(&open_seg)
        )]
    );
    assert_eq!(fs::metadata(&open_seg).expect("open.seg").len(), kept);
}

#[test]
fn a_deleted_cache_is_rebuilt_and_the_next_run_keeps_nothing() {
    let sandbox = Sandbox::new("deleted_cache");
    for name in ["a.md", "b.md", "c.md"] {
        sandbox.write(name, format!("{name}\n"));
    }
    assert_eq!(sandbox.remember_notes().counts(), counts(3, 0, 0, 0, 0));
    fs::remove_dir_all(sandbox.layout().cache).expect("remove the cache");

    let run = sandbox.remember_notes();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.notices,
        [
            "bilbo: rebuilt the catalog from the store: 3 entries in 0 segments",
            "bilbo: building the search index: 3 entries"
        ]
    );
    assert_eq!(run.counts(), counts(0, 0, 0, 3, 0));
    assert_eq!(run.totals(), "records 0, raw bytes 0, stored bytes 0");
}

#[test]
fn a_second_run_waits_on_the_lock_and_says_so() {
    let sandbox = Sandbox::new("lock_wait");
    sandbox.write("a.md", "a\n");
    let layout = sandbox.layout();
    fs::create_dir_all(&layout.state).expect("state folder");
    let lock = fs::File::create(layout.state.join("store.lock")).expect("lock file");
    lock.lock().expect("take the lock");

    let (sender, receiver) = mpsc::channel();
    let vars = sandbox.vars();
    let notes = sandbox.notes();
    let runner = thread::spawn(move || run(vars, &["remember", text(&notes)], &[], Some(sender)));

    let notice = receiver.recv_timeout(WAIT).expect("a waiting notice");
    assert_eq!(
        notice,
        "bilbo: waiting for another bilbo to finish writing the store"
    );
    lock.unlock().expect("release the lock");
    let finished = runner.join().expect("the run finishes");
    assert_eq!(finished.code, 0);
    assert_eq!(
        finished.stdout.lines().next(),
        Some(counts(1, 0, 0, 0, 0).as_str())
    );
}

#[test]
fn a_run_leaves_a_young_open_seg_open_and_seals_one_older_than_seal_after() {
    let sandbox = Sandbox::new("sealed");
    sandbox.write("a.md", "a\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    assert!(sandbox.segments().is_empty());
    assert!(fs::metadata(sandbox.open_seg()).expect("open.seg").len() > 12);
    backdate(&sandbox.layout().state, 61);
    assert_eq!(sandbox.remember_notes().counts(), counts(0, 0, 0, 1, 0));
    let segments = sandbox.segments();
    assert_eq!(segments.len(), 1, "{segments:?}");
    let bytes = fs::read(&segments[0]).expect("read the segment");
    assert!(bytes.starts_with(b"BILBOSEG\x02\0\0\0"));
    assert_eq!(
        fs::metadata(sandbox.open_seg()).expect("open.seg").len(),
        12
    );
    assert!(!sandbox.layout().state.join("open.since").exists());
}

#[test]
fn nothing_to_keep_exits_1_and_creates_no_store() {
    let sandbox = Sandbox::new("nothing_to_keep");
    sandbox.write("x.txt", "text\n");
    let run = sandbox.remember_notes();
    assert_eq!(run.code, 1);
    assert_eq!(run.counts(), counts(0, 0, 0, 0, 0));
    assert_eq!(run.totals(), "records 0, raw bytes 0, stored bytes 0");
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: nothing to keep: no kind claims a file under {}",
            text(&sandbox.notes())
        )]
    );
    let layout = sandbox.layout();
    for folder in [&layout.root, &layout.state, &layout.cache] {
        assert!(!folder.exists(), "{folder:?}");
    }
}

#[test]
fn a_missing_target_is_refused() {
    let sandbox = Sandbox::new("missing_target");
    let missing = sandbox.dir.join("nowhere");
    let run = sandbox.remember(&[text(&missing)]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: cannot read {}: No such file or directory (os error 2)",
            text(&missing)
        )]
    );
}

#[test]
fn the_captured_files_are_never_written() {
    let sandbox = Sandbox::new("captures_untouched");
    sandbox.write("a.md", format!("---\nid: {ID_A}\n---\nbody\n"));
    sandbox.write("b.md", "no newline");
    sandbox.write("sub/c.md", "c\n");
    let before = captured_state(&sandbox.notes());
    for _ in 0..3 {
        assert_eq!(sandbox.remember_notes().code, 0);
    }
    assert_eq!(captured_state(&sandbox.notes()), before);
}

fn captured_state(folder: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    fn walk(folder: &Path, into: &mut BTreeMap<PathBuf, (Vec<u8>, SystemTime)>) {
        for entry in fs::read_dir(folder).expect("list") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, into);
            } else {
                let modified = fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .expect("mtime");
                into.insert(path.clone(), (fs::read(&path).expect("read"), modified));
            }
        }
    }
    let mut into = BTreeMap::new();
    walk(folder, &mut into);
    into
}

#[test]
fn the_disk_lists_sorted_names_and_does_not_follow_links() {
    let sandbox = Sandbox::new("disk");
    let root = sandbox.notes();
    fs::create_dir_all(root.join("sub")).expect("folder");
    sandbox.write("b.md", "b");
    sandbox.write("a.md", "a");
    symlink(root.join("a.md"), root.join("link.md")).expect("link");

    let names: Vec<String> = Disk
        .list(&root)
        .expect("list")
        .into_iter()
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["a.md", "b.md", "link.md", "sub"]);
    assert_eq!(Disk.node(&root.join("a.md")).ok(), Some(Node::File));
    assert_eq!(Disk.node(&root.join("sub")).ok(), Some(Node::Folder));
    assert_eq!(Disk.node(&root.join("link.md")).ok(), Some(Node::Other));
    assert_eq!(Disk.read(&root.join("b.md")).expect("read"), b"b");
    assert_eq!(
        Disk.node(&root.join("none")).map_err(|e| e.kind()),
        Err(io::ErrorKind::NotFound)
    );
}

#[test]
fn a_failed_close_keeps_the_runs_output_and_exits_1() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("failed_close");
    sandbox.write("a.md", "a\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(1, 0, 0, 0, 0));
    sandbox.write("b.md", "b\n");
    backdate(&sandbox.layout().state, 61);
    let root = sandbox.layout().root;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).expect("lock the root");
    let run = sandbox.remember_notes();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("unlock the root");

    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 1, 0));
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(
        run.stderr[0].starts_with("bilbo: cannot "),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_config_error_with_a_file_to_keep_exits_2() {
    let sandbox = Sandbox::new("config_error");
    sandbox.write("a.md", "a\n");
    let mut vars = sandbox.vars();
    vars.insert("BILBO_HOME".to_owned(), "relative".into());
    let notes = sandbox.notes();
    let run = run(vars, &["remember", text(&notes)], &[], None);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        ["bilbo: BILBO_HOME is not an absolute path: relative"]
    );
}

#[test]
fn a_store_that_cannot_open_exits_1_with_the_stores_message() {
    let sandbox = Sandbox::new("open_fails");
    sandbox.write("a.md", "a\n");
    let blocker = sandbox.dir.join("blocker");
    fs::write(&blocker, "a file, not a folder").expect("blocker");
    let mut vars = sandbox.vars();
    vars.insert(
        "BILBO_HOME".to_owned(),
        blocker.join("store").into_os_string(),
    );
    let notes = sandbox.notes();
    let run = run(vars, &["remember", text(&notes)], &[], None);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    let store = blocker.join("store");
    assert!(
        run.stderr[0].starts_with("bilbo: cannot ")
            && run.stderr[0].contains(&*store.to_string_lossy()),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_store_failure_while_keeping_exits_1_with_the_stores_message() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("keep_fails");
    sandbox.write("a.md", "a\n");
    assert_eq!(sandbox.remember_notes().code, 0);
    sandbox.write("b.md", "b\n");
    let catalog = sandbox.layout().cache.join("catalog.sqlite");
    fs::set_permissions(&catalog, fs::Permissions::from_mode(0o400)).expect("lock the catalog");
    let run = sandbox.remember_notes();
    fs::set_permissions(&catalog, fs::Permissions::from_mode(0o600)).expect("unlock the catalog");
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    let prefix = format!("bilbo: cannot use the catalog {}: ", text(&catalog));
    assert!(
        run.stderr[0].starts_with(&prefix) && run.stderr[0].contains("readonly"),
        "{:?}",
        run.stderr
    );
    assert!(
        fs::metadata(sandbox.open_seg()).expect("open.seg").len() > 12,
        "the failed run must leave its frames unsealed"
    );
}

#[test]
fn a_named_link_is_claimed_by_the_resolved_file_not_the_name_given() {
    let sandbox = Sandbox::new("link_extension");
    let plain = sandbox.write("plain", "text\n");
    let link = sandbox.dir.join("note.md");
    symlink(&plain, &link).expect("link");
    let run = sandbox.remember(&[text(&link)]);
    assert_eq!(run.code, 1);
    assert_eq!(run.counts(), counts(0, 0, 0, 0, 0));
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: nothing to keep: no kind claims a file under {}",
            text(&plain)
        )]
    );
}

#[test]
fn a_named_kind_that_claims_nothing_says_so() {
    let sandbox = Sandbox::new("kind_claims_nothing");
    sandbox.write("x.txt", "text\n");
    let notes = sandbox.notes();
    let run = sandbox.remember(&[text(&notes), "--kind", "markdown"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.counts(), counts(0, 0, 0, 0, 0));
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: nothing to keep: markdown claims no file under {}",
            text(&notes)
        )]
    );
}

#[test]
fn a_file_name_with_a_newline_cannot_forge_a_diagnostic_line() {
    let sandbox = Sandbox::new("newline_name");
    sandbox.write("a.md", "fine\n");
    sandbox.write("x\nbilbo: forged.md", [0xff, 0xfe]);
    let run = sandbox.remember_notes();
    assert_eq!(run.code, 1);
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(
        run.stderr[0].contains("x\\nbilbo: forged.md"),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_missing_target_whose_name_holds_a_newline_stays_one_line() {
    let sandbox = Sandbox::new("newline_target");
    let missing = sandbox.dir.join("no\nbilbo: forged");
    let run = sandbox.remember(&[text(&missing)]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(
        run.stderr[0].contains("no\\nbilbo: forged"),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_store_path_that_holds_a_newline_stays_one_line() {
    let sandbox = Sandbox::new("newline_store");
    sandbox.write("a.md", "a\n");
    let blocker = sandbox.dir.join("blocker");
    fs::write(&blocker, "a file").expect("blocker");
    let mut vars = sandbox.vars();
    vars.insert(
        "BILBO_HOME".to_owned(),
        blocker.join("st\nbilbo: forged").into_os_string(),
    );
    let notes = sandbox.notes();
    let run = run(vars, &["remember", text(&notes)], &[], None);
    assert_eq!(run.code, 1);
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(
        run.stderr[0].contains("st\\nbilbo: forged"),
        "{:?}",
        run.stderr
    );
}

#[test]
fn a_notice_naming_a_path_with_a_newline_stays_one_line() {
    let sandbox = Sandbox::new("newline_notice");
    sandbox.write("a.md", "a\n");
    let mut vars = sandbox.vars();
    vars.insert(
        "XDG_STATE_HOME".to_owned(),
        sandbox.dir.join("st\nbilbo: forged").into_os_string(),
    );
    let notes = sandbox.notes();
    assert_eq!(
        run(vars.clone(), &["remember", text(&notes)], &[], None).code,
        0
    );
    let layout = layout(&vars);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(layout.state.join("open.seg"))
        .expect("open.seg");
    file.write_all(&[0xff, 0, 0, 0]).expect("tear");
    file.write_all(&[7; 16]).expect("junk");
    drop(file);

    let run = run(vars, &["remember", text(&notes)], &[], None);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.notices.len(), 1, "{:?}", run.notices);
    assert!(
        run.notices[0].contains("st\\nbilbo: forged"),
        "{:?}",
        run.notices
    );
}

#[test]
fn a_config_path_with_a_dotted_component_exits_2_and_creates_nothing() {
    let sandbox = Sandbox::new("dotted_home");
    sandbox.write("a.md", "a\n");
    let dotted = format!("{}/store/../store", sandbox.dir.display());
    let mut vars = sandbox.vars();
    vars.insert("BILBO_HOME".to_owned(), dotted.clone().into());
    let notes = sandbox.notes();
    let run = run(vars, &["remember", text(&notes)], &[], None);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        [format!(
            "bilbo: BILBO_HOME has a . or .. component: {dotted}"
        )]
    );
    let layout = sandbox.layout();
    for folder in [&layout.root, &layout.state, &layout.cache] {
        assert!(!folder.exists(), "{folder:?}");
    }
}

#[test]
fn a_root_that_holds_0_20s_notes_exits_2_and_writes_nothing_even_with_nothing_to_keep() {
    for files in [false, true] {
        let sandbox = Sandbox::new("older_root");
        if files {
            sandbox.write("a.md", "a\n");
        }
        let layout = sandbox.layout();
        fs::create_dir_all(layout.root.join("notes")).expect("a 0.20 notes folder");
        let run = sandbox.remember_notes();
        assert_eq!(run.code, 2, "files: {files}");
        assert_eq!(run.stdout, "", "files: {files}");
        assert_eq!(
            run.stderr,
            [format!(
                "bilbo: {}/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder",
                layout.root.display()
            )],
            "files: {files}"
        );
        assert!(
            !layout.state.exists() && !layout.cache.exists(),
            "files: {files}"
        );
        assert_eq!(
            fs::read_dir(&layout.root).expect("the root").count(),
            1,
            "only notes/, files: {files}"
        );
    }
}

const USAGE_LINES: [&str; 6] = [
    "bilbo: usage: bilbo remember <file|folder> [--kind <kind>] [--tag <tag>]...",
    "bilbo:        [--follow]",
    "bilbo:        bilbo remember - [--tag <tag>]...",
    "bilbo:        bilbo remember --edit [--tag <tag>]...",
    "bilbo:        bilbo remember",
    "bilbo: see 'bilbo remember --help'",
];

fn with_usage(reason: &str) -> Vec<String> {
    std::iter::once(reason)
        .chain(USAGE_LINES)
        .map(str::to_owned)
        .collect()
}

impl Sandbox {
    fn remember_tagged(&self, tags: &[&str]) -> Run {
        let notes = self.notes();
        let mut args = vec![text(&notes)];
        for tag in tags {
            args.extend(["--tag", tag]);
        }
        self.remember(&args)
    }

    fn tags(&self, file: &Path) -> Vec<String> {
        let (meta, _) = self.entry(file);
        meta.labels
            .tags
            .carried()
            .map(|(tag, _)| tag.to_string())
            .collect()
    }
}

#[test]
fn remember_with_a_tag_says_tagged_and_new_and_the_entry_carries_the_tag() {
    let sandbox = Sandbox::new("tag_says_tagged");
    let file = sandbox.write("a.md", "one\n");

    let run = sandbox.remember_tagged(&["rust"]);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
    assert_eq!(run.stdout.lines().count(), 2);
    assert_eq!(
        run.stderr,
        ["bilbo: tagged 1 entries", "bilbo: tag:rust is new"]
    );
    assert_eq!(sandbox.tags(&file), ["tag:rust"]);

    sandbox.write("b.md", "two\n");
    let more = sandbox.remember_tagged(&["rust", "garden"]);
    assert_eq!(more.code, 0, "{:?}", more.stderr);
    assert_eq!(
        more.stderr,
        ["bilbo: tagged 2 entries", "bilbo: tag:garden is new"]
    );

    sandbox.write("c.md", "three\n");
    let mixed = sandbox.remember_tagged(&["rust", "garden"]);
    assert_eq!(mixed.code, 0, "{:?}", mixed.stderr);
    assert_eq!(mixed.stderr, ["bilbo: tagged 1 entries"]);
}

#[test]
fn a_second_run_with_a_tag_keeps_nothing_and_tags_what_was_not_tagged() {
    let sandbox = Sandbox::new("tag_second_run");
    sandbox.write("a.md", "one\n");
    sandbox.write("b.md", "two\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(2, 0, 0, 0, 0));

    let second = sandbox.remember_tagged(&["docs"]);
    assert_eq!(second.code, 0, "{:?}", second.stderr);
    assert_eq!(second.counts(), counts(0, 0, 0, 2, 0));
    assert_eq!(
        second.stderr,
        ["bilbo: tagged 2 entries", "bilbo: tag:docs is new"]
    );

    let third = sandbox.remember_tagged(&["docs"]);
    assert_eq!(third.code, 0, "{:?}", third.stderr);
    assert_eq!(third.counts(), counts(0, 0, 0, 2, 0));
    assert_eq!(third.stderr, Vec::<String>::new());
}

#[test]
fn a_tag_that_is_no_tags_name_is_a_usage_error() {
    let sandbox = Sandbox::new("tag_not_a_name");
    sandbox.write("a.md", "one\n");

    let run = sandbox.remember_tagged(&["Rust"]);

    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        with_usage(
            "bilbo: --tag 'Rust' is not a tag: parts of a to z and digits joined by single hyphens, at most 64 bytes"
        )
    );
    assert!(!sandbox.layout().root.exists());
}

#[test]
fn a_tag_does_not_stand_with_follow_and_needs_a_target() {
    let sandbox = Sandbox::new("tag_follow");
    sandbox.write("a.md", "one\n");
    let notes = sandbox.notes();
    let config = sandbox.dir.join("config/config.toml");

    let together = sandbox.remember(&[text(&notes), "--tag", "a", "--follow"]);
    assert_eq!(together.code, 2);
    assert_eq!(
        together.stderr,
        with_usage("bilbo: --tag does not stand with --follow: a follow keeps no tag")
    );
    assert_eq!(fs::read_to_string(&config).expect("config"), "");

    let bare = sandbox.remember(&["--tag", "a"]);
    assert_eq!(bare.code, 2);
    assert_eq!(bare.stderr[0], BARE);
}

#[test]
fn an_empty_tag_needs_a_value() {
    let sandbox = Sandbox::new("tag_empty");
    let notes = sandbox.notes();

    let run = sandbox.remember(&[text(&notes), "--tag="]);

    assert_eq!(run.code, 2);
    assert_eq!(run.stderr[0], "bilbo: --tag needs a value");
}

#[test]
fn a_file_past_the_cap_is_refused_with_its_reason_and_exit_1() {
    let sandbox = Sandbox::new("tag_cap");
    let file = sandbox.write("a.md", "one\n");
    let notes = sandbox.notes();
    let mut vars = sandbox.vars();
    vars.insert("BILBO_TAG_CAP".to_owned(), "1".into());
    let first = run(
        vars.clone(),
        &["remember", text(&notes), "--tag", "a"],
        &[],
        None,
    );
    assert_eq!(first.code, 0, "{:?}", first.stderr);
    let (_, kept) = sandbox.entry(&file);
    fs::write(&file, "one\ntwo\n").expect("grow");

    let refused = run(vars, &["remember", text(&notes), "--tag", "b"], &[], None);

    assert_eq!(refused.code, 1);
    assert_eq!(refused.counts(), counts(0, 0, 0, 0, 1));
    assert_eq!(
        refused.stderr,
        [format!(
            "bilbo: refused {}: its entry would carry more than tag.cap tags (1)",
            text(&file)
        )]
    );
    let (_, head) = sandbox.entry(&file);
    assert_eq!(
        (head.generation, head.records),
        (kept.generation, kept.records)
    );
    assert_eq!(sandbox.tags(&file), ["tag:a"]);
}

#[test]
fn edit_stands_with_tags_alone_and_needs_an_editor() {
    let sandbox = Sandbox::new("edit_grammar");
    let temp = sandbox.dir.join("tmp");
    fs::create_dir_all(&temp).expect("a temp folder");
    let with_temp = || {
        let mut vars = sandbox.vars();
        vars.insert("TMPDIR".to_owned(), temp.clone().into_os_string());
        vars
    };
    let call = |args: &[&str]| run(with_temp(), &[&["remember"][..], args].concat(), &[], None);

    for args in [&["--edit", "notes.md"][..], &["--edit", "-"]] {
        let run = call(args);
        assert_eq!(
            (run.code, run.stderr),
            (
                2,
                with_usage(
                    "bilbo: --edit does not stand with a target: it keeps a note written in the editor"
                )
            ),
            "{args:?}"
        );
    }
    for args in [
        &["--edit", "--kind", "markdown"][..],
        &["--edit", "--follow"],
    ] {
        let run = call(args);
        assert_eq!(
            (run.code, run.stderr),
            (2, with_usage("bilbo: missing <file|folder>")),
            "{args:?}"
        );
    }
    let tag = call(&["--tag", "garden"]);
    assert_eq!((tag.code, tag.stderr), (2, with_usage(BARE)));

    let none = call(&["--edit"]);
    assert_eq!(none.code, 2);
    assert_eq!(none.stdout, "");
    assert_eq!(
        none.stderr,
        ["bilbo: --edit needs an editor: set remember.editor, VISUAL or EDITOR"]
    );
    assert_eq!(fs::read_dir(&temp).expect("the temp folder").count(), 0);
    sandbox.no_store_folders();

    let mut vars = sandbox.vars();
    vars.insert(
        "TMPDIR".to_owned(),
        sandbox.dir.join("not-there").into_os_string(),
    );
    let absent = run(vars, &["remember", "--edit"], &[], None);
    assert_eq!(absent.code, 2);
    assert_eq!(absent.stdout, "");
    assert_eq!(
        absent.stderr,
        ["bilbo: --edit needs an editor: set remember.editor, VISUAL or EDITOR"]
    );
    sandbox.no_store_folders();
}

fn question_lines() -> Vec<String> {
    [
        "bilbo: What should bilbo keep?",
        "bilbo: 1  a note typed here",
        "bilbo: 2  a note written in the editor",
        "bilbo: 3  a file or folder",
        "bilbo: Type 1, 2 or 3, then Enter. An empty answer keeps nothing.",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn question_then(lines: &[&str]) -> Vec<String> {
    let mut all = question_lines();
    all.extend(lines.iter().map(|line| (*line).to_owned()));
    all
}

fn as_a_person(sandbox: &Sandbox, args: &[&str], input: &[u8]) -> Run {
    run_as(
        sandbox.vars(),
        &[&["remember"][..], args].concat(),
        Who::Person,
        Box::new(io::Cursor::new(input.to_vec())),
        None,
    )
}

#[test]
fn a_note_typed_at_the_prompt_is_kept() {
    let sandbox = Sandbox::new("prompt_typed");
    let run = as_a_person(&sandbox, &[], b"1\nThe beet seeds are in the drawer.\n");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
    let id = kept_id(&run);
    assert_eq!(
        run.notices,
        question_then(&["bilbo: Type the note, then Ctrl-D on a line of its own."])
    );
    assert_eq!(run.stderr, [format!("bilbo: kept the note as {id}")]);
    let entries = sandbox.entries();
    assert_eq!(entries.len(), 1);
    let (meta, head) = &entries[0];
    assert_eq!(meta.id.to_string(), id);
    assert_eq!(meta.kind, "note");
    assert_eq!(
        meta.labels.capture.as_ref().map(ToString::to_string),
        Some(format!("note:{id}"))
    );
    assert_eq!(head.records, 1);
    let found = run_in(&sandbox, &["recall", "--", "beet"]);
    assert_eq!(found.code, 0, "{:?}", found.stderr);
    let headers = hit_headers(&found.stdout);
    assert_eq!(headers.len(), 1, "{}", found.stdout);
    assert_eq!(headers[0][0], format!("{}#r1", &id[id.len() - 10..]));
    assert_eq!(headers[0][2], "note", "{headers:?}");
}

#[test]
fn a_file_or_folder_named_at_the_prompt_is_kept_as_a_target() {
    let asked = Sandbox::new("prompt_file");
    let file = asked.write("garden.md", "# Garden\n\nThe beds face south.\n");
    asked.write("other.md", "# Garden\n\nThe beds face north.\n");
    let answers = format!("3\n{}\n", text(&file));
    let run = as_a_person(&asked, &[], answers.as_bytes());
    assert_eq!(run.code, 0, "{:?}", run.stderr);

    let direct = Sandbox::new("prompt_file_direct");
    let same = direct.write("garden.md", "# Garden\n\nThe beds face south.\n");
    direct.write("other.md", "# Garden\n\nThe beds face north.\n");
    let target = direct.remember(&[text(&same)]);
    assert_eq!(run.counts(), target.counts());
    assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
    assert_eq!(run.stdout.lines().count(), 2, "{}", run.stdout);
    let stored: u64 = run
        .totals()
        .strip_prefix("records 3, raw bytes 31, stored bytes ")
        .and_then(|number| number.parse().ok())
        .expect("a whole number of stored bytes");
    assert!(stored > 0);
    assert_eq!(
        run.totals()
            .rsplit_once(", stored bytes ")
            .map(|(kept, _)| kept),
        target
            .totals()
            .rsplit_once(", stored bytes ")
            .map(|(kept, _)| kept)
    );
    assert_eq!(run.stderr, target.stderr);
    let (kept, direct_kept) = (asked.entries(), direct.entries());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept.len(), direct_kept.len());
    let (meta, head) = asked.entry(&file);
    assert_eq!(meta.kind, "markdown");
    assert_eq!(head.records, 3);
    let records = asked.generation(meta.id, head.generation, 3);
    assert_eq!(records, b"# Garden\n\nThe beds face south.\n");
    assert_eq!(kept[0].0.kind, direct_kept[0].0.kind);
    assert_eq!(
        run.notices,
        question_then(&["bilbo: Which file or folder?"])
    );
}

#[test]
fn an_empty_answer_or_the_end_of_input_keeps_nothing() {
    for input in [&b"\n"[..], b""] {
        let sandbox = Sandbox::new("prompt_nothing");
        let run = as_a_person(&sandbox, &[], input);
        assert_eq!(run.code, 0, "{input:?}: {:?}", run.stderr);
        assert_eq!(run.stdout, "", "{input:?}");
        assert_eq!(run.notices, question_lines(), "{input:?}");
        assert_eq!(run.stderr, ["bilbo: nothing kept"], "{input:?}");
        sandbox.no_store_folders();
    }
}

#[test]
fn a_bare_remember_with_nobody_at_the_terminal_or_with_an_option_never_asks() {
    let sandbox = Sandbox::new("prompt_never");
    let nobody = sandbox.piped(&[], b"1\nThe beet seeds are in the drawer.\n");
    assert_eq!((nobody.code, nobody.stderr), (2, with_usage(BARE)));
    assert_eq!(nobody.stdout, "");
    assert!(nobody.notices.is_empty(), "{:?}", nobody.notices);
    let tag = as_a_person(&sandbox, &["--tag", "garden"], b"1\nA note.\n");
    assert_eq!((tag.code, tag.stderr), (2, with_usage(BARE)));
    assert_eq!(tag.stdout, "");
    assert!(tag.notices.is_empty(), "{:?}", tag.notices);
    sandbox.no_store_folders();
}

#[test]
fn the_editors_answer_with_no_editor_is_the_editors_error() {
    let sandbox = Sandbox::new("prompt_editor");
    let run = as_a_person(&sandbox, &[], b"2\n");
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(run.notices, question_lines());
    assert_eq!(
        run.stderr,
        ["bilbo: --edit needs an editor: set remember.editor, VISUAL or EDITOR"]
    );
    sandbox.no_store_folders();
}

#[test]
fn a_run_that_keeps_a_written_note_serves_another_bilbos_request_for_a_pass() {
    let served = "bilbo: ran the follows again for another bilbo: new 1, appended 0, new generation 0, unchanged 0, refused 0";
    for (row, input) in [
        (1, "The beet seeds are in the drawer.\n".to_owned()),
        (2, "z".repeat(256)),
    ] {
        let sandbox = Sandbox::new("piped_served");
        let followed = sandbox.write("a.md", "# Garden\n");
        write(
            &sandbox.dir.join("config/config.toml"),
            format!(
                "follow = {}\n",
                show_follows(&[Follow {
                    path: sandbox.notes(),
                    kind: Some("markdown".to_owned()),
                }])
            ),
        );
        write(&sandbox.layout().state.join("store.again"), b"1");
        let run = sandbox.piped(&["-"], input.as_bytes());
        let entries = sandbox.entries();
        if row == 1 {
            assert_eq!(run.code, 0, "{:?}", run.stderr);
            assert_eq!(run.counts(), counts(1, 0, 0, 0, 0));
            assert_eq!(
                run.stderr,
                [
                    format!("bilbo: kept the note as {}", kept_id(&run)),
                    served.to_owned()
                ]
            );
            let mut kinds: Vec<&str> = entries.iter().map(|(meta, _)| meta.kind.as_str()).collect();
            kinds.sort_unstable();
            assert_eq!(kinds, ["markdown", "note"]);
            let note = entries
                .iter()
                .find(|(meta, _)| meta.kind == "note")
                .expect("the note");
            assert_eq!(note.0.id.to_string(), kept_id(&run));
        } else {
            assert_eq!(run.code, 1, "{:?}", run.stderr);
            assert_eq!(run.counts(), counts(0, 0, 0, 0, 1));
            assert_eq!(
                run.stderr,
                [
                    "bilbo: refused the note: the store would keep only a placeholder for its text, which it takes for encoded data".to_owned(),
                    served.to_owned()
                ]
            );
            assert_eq!(entries.len(), 1);
        }
        let (meta, head) = sandbox.entry(&followed);
        assert_eq!(meta.kind, "markdown", "row {row}");
        assert_eq!(head.records, 1, "row {row}");
    }
}
