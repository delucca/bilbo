use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::binary;
use super::scratch::{NOTE, SESSION, Scratch};

const WAIT: Duration = Duration::from_secs(20);

fn remembered_from_home(test: &str) -> Scratch {
    let scratch = Scratch::new(test);
    let session = scratch.dir.join(
        "home/.claude/projects/-home-me-src-garden/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl",
    );
    fs::create_dir_all(session.parent().expect("a parent")).expect("folders");
    fs::write(&session, SESSION).expect("a session");
    scratch.write("sections.md", NOTE);
    for target in [scratch.path("home/.claude/projects"), scratch.path("notes")] {
        let run = scratch.run(&["remember", &target]);
        assert_eq!(run.code, 0, "{}", run.stderr);
    }
    scratch
}

fn counts(stdout: &str) -> [u64; 4] {
    let (first, second) = stdout.split_once('\n').expect("two lines");
    let numbers = |line: &str, names: &[&str]| -> Vec<u64> {
        let fields: Vec<&str> = line.split(", ").collect();
        assert_eq!(fields.len(), names.len(), "{line:?}");
        fields
            .iter()
            .zip(names)
            .map(|(field, name)| {
                let number = field
                    .strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix(' '));
                number
                    .unwrap_or_else(|| panic!("{line:?}"))
                    .parse()
                    .expect("a number")
            })
            .collect()
    };
    let second = second.strip_suffix('\n').expect("a final line feed");
    assert!(numbers(second, &["index bytes", "milliseconds"])[0] > 0);
    let first = numbers(first, &["entries", "chunks", "unread", "failed"]);
    [first[0], first[1], first[2], first[3]]
}

#[test]
fn a_remembered_store_is_indexed_and_a_deleted_cache_folder_comes_back() {
    let scratch = remembered_from_home("reindex");
    let first = scratch.run(&["reindex"]);
    assert_eq!(
        (first.code, first.stderr.as_str()),
        (0, ""),
        "{}",
        first.stdout
    );
    let [entries, chunks, unread, failed] = counts(&first.stdout);
    assert_eq!((entries, unread, failed), (2, 0, 0));
    assert!(chunks >= 2, "{chunks}");
    let cache = scratch.dir.join("cache/bilbo");
    assert!(cache.join("catalog.sqlite").is_file());
    assert!(cache.join("index/meta.json").is_file());

    let read = scratch.run(&["read", "8f2b6d4e"]);
    assert_eq!(read.code, 0, "{}", read.stderr);

    fs::remove_dir_all(scratch.dir.join("cache")).expect("delete the cache folder");
    let second = scratch.run(&["reindex"]);
    assert_eq!((second.code, second.stderr.as_str()), (0, ""));
    assert_eq!(counts(&second.stdout), [entries, chunks, 0, 0]);
    assert!(cache.join("index/meta.json").is_file());
    let again = scratch.run(&["read", "8f2b6d4e"]);
    assert_eq!((again.code, again.stdout), (0, read.stdout));

    let unchanged = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(unchanged.code, 0, "{}", unchanged.stderr);
    assert!(
        unchanged
            .stdout
            .starts_with("new 0, appended 0, new generation 0, unchanged 1, refused 0\n"),
        "{}",
        unchanged.stdout
    );
}

#[test]
fn reindex_takes_no_argument_and_shares_every_verbs_exit_codes() {
    let scratch = remembered_from_home("reindex_exits");
    let extra = scratch.run(&["reindex", "now"]);
    assert_eq!((extra.code, extra.stdout.as_str()), (2, ""));
    assert_eq!(
        extra.stderr,
        "bilbo: unexpected argument 'now'\nbilbo: usage: bilbo reindex\nbilbo: see 'bilbo reindex --help'\n"
    );
    let option = scratch.run(&["reindex", "--force"]);
    assert_eq!((option.code, option.stdout.as_str()), (2, ""));
    assert!(
        option
            .stderr
            .starts_with("bilbo: unknown option '--force'\n"),
        "{}",
        option.stderr
    );

    scratch.config("[search]\nchunk_tokens = 12\n");
    let config = scratch.run(&["reindex"]);
    assert_eq!((config.code, config.stdout.as_str()), (2, ""));
    assert!(
        config
            .stderr
            .contains("search.chunk_tokens must be a whole number from 256 to 8192, got 12"),
        "{}",
        config.stderr
    );
    scratch.config("");

    let index = scratch.dir.join("cache/bilbo/index");
    fs::create_dir_all(&index).expect("the index folder");
    fs::write(index.join("mine.txt"), "not bilbo's").expect("a file of someone else's");
    let foreign = scratch.run(&["reindex"]);
    assert_eq!((foreign.code, foreign.stdout.as_str()), (1, ""));
    assert_eq!(
        foreign.stderr,
        format!(
            "bilbo: {} holds files that are not bilbo's index; bilbo will not touch it\n",
            index.display()
        )
    );
    assert_eq!(
        fs::read_to_string(index.join("mine.txt")).expect("the file"),
        "not bilbo's"
    );

    let help = scratch.run(&["reindex", "--help"]);
    assert_eq!((help.code, help.stderr.as_str()), (0, ""));
    assert!(
        help.stdout
            .starts_with("bilbo reindex: rebuild the catalog and the search index")
    );
}

#[test]
fn a_reindex_waits_for_a_writer_and_a_bare_remember_does_not_wait_for_a_reindex() {
    let scratch = remembered_from_home("reindex_lock");
    let state = scratch.dir.join("state/bilbo");
    let lock = fs::File::open(state.join("store.lock")).expect("the lock file");
    lock.lock().expect("hold the store");
    let writer = fs::File::open(state.join("store.writer")).expect("the writer's lock file");
    writer.lock().expect("as a writer does");

    let env = scratch.env();
    let mut child = Command::new(binary())
        .env_clear()
        .envs(env.iter().map(|(name, value)| (*name, value.as_str())))
        .arg("reindex")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bilbo runs");
    let mut stderr = child.stderr.take().expect("stderr");
    let mut said = Vec::new();
    let mut byte = [0_u8; 1];
    while !said.ends_with(b"\n") {
        stderr.read_exact(&mut byte).expect("the waiting line");
        said.push(byte[0]);
    }
    assert_eq!(
        String::from_utf8_lossy(&said),
        "bilbo: waiting for another bilbo to finish writing the store\n"
    );
    assert!(child.try_wait().expect("a state").is_none(), "it waits");

    let bare = scratch.run(&["remember"]);
    assert_eq!((bare.code, bare.stdout.as_str()), (0, ""));

    writer.unlock().expect("release");
    lock.unlock().expect("release");
    let deadline = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = child.try_wait().expect("a state") {
            break status;
        }
        assert!(Instant::now() < deadline, "reindex did not finish");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut stdout)
        .expect("UTF-8");
    assert_eq!(counts(&stdout)[0], 2);
    assert_eq!(
        fs::metadata(state.join("store.again"))
            .expect("the flag")
            .len(),
        0,
        "reindex served the request"
    );
}

/// The byte where the sstable version of the `.fast` file of an index made of `NOTE` alone sits,
/// found by flipping each byte and reading what a run says: tantivy then refuses the segment with
/// an error that is not a broken file's.
const SSTABLE_VERSION_AT: usize = 405;

#[test]
fn a_reindex_repairs_an_index_an_update_could_not_read() {
    let scratch = Scratch::new("reindex_repairs");
    scratch.write("sections.md", NOTE);
    let first = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!((first.code, first.stderr.as_str()), (0, ""));

    let index = scratch.dir.join("cache/bilbo/index");
    let fast = fs::read_dir(&index)
        .expect("the index folder")
        .map(|entry| entry.expect("an entry").path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "fast")
        })
        .expect("a .fast file");
    let mut bytes = fs::read(&fast).expect("read the .fast file");
    bytes[SSTABLE_VERSION_AT] ^= 0x55;
    fs::write(&fast, bytes).expect("damage the .fast file");

    scratch.write("other.md", "# Other\n\nanother note about kiwis\n");
    let failed = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(failed.code, 1, "{}", failed.stderr);
    assert!(
        failed
            .stderr
            .starts_with("bilbo: the search index was not updated: cannot read the index "),
        "the damage no longer reads as an index that cannot be looked at: {}",
        failed.stderr
    );
    assert!(
        failed
            .stderr
            .ends_with("; bilbo reindex makes the index again\n"),
        "{}",
        failed.stderr
    );
    assert!(failed.stdout.starts_with("new 1, "), "{}", failed.stdout);

    let repaired = scratch.run(&["reindex"]);
    assert_eq!((repaired.code, repaired.stderr.as_str()), (0, ""));
    assert_eq!(counts(&repaired.stdout)[0], 2);

    scratch.write("third.md", "# Third\n\nand a third one about leeks\n");
    let next = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!((next.code, next.stderr.as_str()), (0, ""));
}

/// A byte inside a column of the `.fast` file of an index made of `NOTE` alone: the segment opens
/// and the damage shows only when the entry's columns are read, where tantivy panics.
const COLUMN_AT: usize = 70;

#[test]
fn a_reindex_repairs_an_index_whose_columns_an_update_could_not_read() {
    let scratch = Scratch::new("reindex_repairs_columns");
    scratch.write("sections.md", NOTE);
    let first = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!((first.code, first.stderr.as_str()), (0, ""));

    let index = scratch.dir.join("cache/bilbo/index");
    let fast = fs::read_dir(&index)
        .expect("the index folder")
        .map(|entry| entry.expect("an entry").path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "fast")
        })
        .expect("a .fast file");
    let mut bytes = fs::read(&fast).expect("read the .fast file");
    bytes[COLUMN_AT] ^= 0x55;
    fs::write(&fast, bytes).expect("damage the .fast file");

    scratch.write(
        "sections.md",
        &format!("{NOTE}\n\nand a grown line about kiwis\n"),
    );
    let failed = scratch.run(&["remember", &scratch.path("notes")]);
    assert_eq!(failed.code, 1, "{}", failed.stderr);
    let last = failed.stderr.lines().last().expect("a last line");
    assert!(
        last.starts_with("bilbo: the search index was not updated: cannot read the index ")
            && last.contains(": its content could not be read; bilbo reindex"),
        "{}",
        failed.stderr
    );
    assert!(
        last.ends_with("; bilbo reindex makes the index again"),
        "{}",
        failed.stderr
    );
    assert!(
        failed.stdout.starts_with("new 0, appended 1, "),
        "{}",
        failed.stdout
    );

    let repaired = scratch.run(&["reindex"]);
    assert_eq!((repaired.code, repaired.stderr.as_str()), (0, ""));
    assert_eq!(counts(&repaired.stdout)[0], 1);
}
