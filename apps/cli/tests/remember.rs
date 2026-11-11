use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::remember::disk::Disk;
use bilbo_kinds::files::{Files, Node};
use bilbo_store::entries::{Cursor, Entries, Generation, Head, Meta, Origin, RecordRange};
use bilbo_store::id::Id;
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Options, Store};

mod sandbox;

use sandbox::{Run, backdate, counts, layout, run, scratch_dir, text, vars, write};

const ID_A: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
const ID_B: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DF";
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
        run(self.vars(), args, b"", None)
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
            let origin = meta.origin.clone().expect("an origin");
            let head = store
                .by_origin(&origin)
                .expect("by_origin")
                .expect("a head");
            found.push((meta, head));
        }
        found
    }

    fn entry(&self, file: &Path) -> (Meta, Head) {
        let origin = Origin::file(file).expect("an origin");
        self.entries()
            .into_iter()
            .find(|(meta, _)| meta.origin.as_ref() == Some(&origin))
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
fn a_named_link_to_a_file_is_taken_and_its_origin_is_the_resolved_path() {
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
            entries[0].0.origin.as_ref().map(Origin::as_str),
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
        let origin = meta.origin.expect("an origin");
        let path = origin.path().expect("a file origin");
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
fn a_file_that_left_the_folder_keeps_its_entry() {
    let sandbox = Sandbox::new("file_left");
    sandbox.write("a.md", "a\n");
    let gone = sandbox.write("b.md", "b\n");
    assert_eq!(sandbox.remember_notes().counts(), counts(2, 0, 0, 0, 0));
    fs::remove_file(&gone).expect("remove");
    let run = sandbox.remember_notes();
    assert_eq!(run.code, 0);
    assert_eq!(run.counts(), counts(0, 0, 0, 1, 0));
    assert_eq!(sandbox.entries().len(), 2);
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
    let runner = thread::spawn(move || run(vars, &[text(&notes)], b"", Some(sender)));

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
    assert!(bytes.starts_with(b"BILBOSEG\x01\0\0\0"));
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
fn the_origin_files_are_never_written() {
    let sandbox = Sandbox::new("origins_untouched");
    sandbox.write("a.md", format!("---\nid: {ID_A}\n---\nbody\n"));
    sandbox.write("b.md", "no newline");
    sandbox.write("sub/c.md", "c\n");
    let before = origin_state(&sandbox.notes());
    for _ in 0..3 {
        assert_eq!(sandbox.remember_notes().code, 0);
    }
    assert_eq!(origin_state(&sandbox.notes()), before);
}

fn origin_state(folder: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
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
    let run = run(vars, &[text(&notes)], b"", None);
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
    let run = run(vars, &[text(&notes)], b"", None);
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
    let run = run(vars, &[text(&notes)], b"", None);
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
    assert_eq!(run(vars.clone(), &[text(&notes)], b"", None).code, 0);
    let layout = layout(&vars);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(layout.state.join("open.seg"))
        .expect("open.seg");
    file.write_all(&[0xff, 0, 0, 0]).expect("tear");
    file.write_all(&[7; 16]).expect("junk");
    drop(file);

    let run = run(vars, &[text(&notes)], b"", None);
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
    let run = run(vars, &[text(&notes)], b"", None);
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
