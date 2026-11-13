use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::{Duration, SystemTime};

use bilbo_cli::console::settings;
use bilbo_cli::remember::save::{Saved, Saving, save_with};
use bilbo_kinds::follows::Follow;
use bilbo_store::entries::{Entries, Generation, Head, Meta, Origin, RecordRange};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Options, Store};

mod sandbox;

use sandbox::{Run, backdate, counts, layout, run, scratch_dir, text, vars, write};

const MAIN: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
const OTHER: &str = "9b1d2c3e-4f5a-4b6c-8d7e-0f1a2b3c4d5e";
const WAIT: Duration = Duration::from_secs(10);
const AGENT: &str = "agent-ad63c2b3c9e7a4b3d";
const ROLLOUT: &str = "rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl";
const CLAUDE: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
);
const SUBAGENT: &[u8] =
    include_bytes!("../../../libs/rust/kinds/session/fixtures/agent-ad63c2b3c9e7a4b3d.jsonl");
const META: &[u8] =
    include_bytes!("../../../libs/rust/kinds/session/fixtures/agent-ad63c2b3c9e7a4b3d.meta.json");
const CODEX: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
);
const CODEX_PACKED: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl.zst"
);
const SECOND: &str = "rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl";
const SECOND_BYTES: &[u8] = include_bytes!(
    "../../../libs/rust/kinds/session/fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
);

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(test: &str) -> Sandbox {
        let sandbox = Sandbox {
            dir: scratch_dir(test),
        };
        write(&sandbox.main(), CLAUDE);
        write(&sandbox.subagent(), SUBAGENT);
        write(&sandbox.subagent().with_extension("meta.json"), META);
        write(&sandbox.project().join("memory/MEMORY.md"), b"- a memory\n");
        for skipped in [
            "memory".to_owned(),
            format!("{MAIN}/tool-results"),
            format!("{MAIN}/workflows"),
        ] {
            write(
                &sandbox
                    .project()
                    .join(skipped)
                    .join(format!("{OTHER}.jsonl")),
                CLAUDE,
            );
        }
        write(&sandbox.workflow_subagent(), SUBAGENT);
        write(&sandbox.rollout(), CODEX);
        write(&sandbox.dir.join("config/config.toml"), b"");
        sandbox
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    fn projects(&self) -> PathBuf {
        self.home().join(".claude/projects")
    }

    fn sessions(&self) -> PathBuf {
        self.home().join(".codex/sessions")
    }

    fn project(&self) -> PathBuf {
        self.projects().join("-home-me-src-garden")
    }

    fn main(&self) -> PathBuf {
        self.project().join(format!("{MAIN}.jsonl"))
    }

    fn subagent(&self) -> PathBuf {
        self.project()
            .join(format!("{MAIN}/subagents/{AGENT}.jsonl"))
    }

    fn workflow_subagent(&self) -> PathBuf {
        self.project()
            .join(format!("{MAIN}/subagents/workflows/wf_1/agent-b2.jsonl"))
    }

    fn rollout(&self) -> PathBuf {
        self.sessions().join("2026/11/06").join(ROLLOUT)
    }

    fn packed_rollout(&self) -> PathBuf {
        let mut name = self.rollout().into_os_string();
        name.push(".zst");
        PathBuf::from(name)
    }

    fn head(&self, file: &Path) -> Head {
        self.store()
            .by_origin(&Origin::file(file).expect("origin"))
            .expect("head")
            .expect("an entry")
    }

    fn leave(&self, plain: Option<&[u8]>, packed: Option<&[u8]>) {
        for (path, bytes) in [(self.rollout(), plain), (self.packed_rollout(), packed)] {
            let _ = fs::remove_file(&path);
            if let Some(bytes) = bytes {
                write(&path, bytes);
            }
        }
    }

    fn keep_sessions(&self) -> Run {
        self.run(&[text(&self.sessions())])
    }

    fn vars(&self) -> HashMap<String, OsString> {
        vars(&self.dir)
    }

    fn layout(&self) -> Layout {
        layout(&self.vars())
    }

    fn run(&self, args: &[&str]) -> Run {
        run(self.vars(), args, None)
    }

    fn run_noticed(&self, args: &[&str], notices: Option<Sender<String>>) -> Run {
        run(self.vars(), args, notices)
    }

    fn store(&self) -> Store {
        Store::open_read_only(&self.layout(), Options::default(), &mut |_| {}).expect("read")
    }

    fn meta(&self, file: &Path) -> Meta {
        let store = self.store();
        let head = store
            .by_origin(&Origin::file(file).expect("origin"))
            .expect("head")
            .unwrap_or_else(|| panic!("no entry for {}", file.display()));
        store.meta(head.id).expect("meta").expect("a meta")
    }

    fn records(&self, file: &Path) -> Vec<Vec<u8>> {
        let store = self.store();
        let head = store
            .by_origin(&Origin::file(file).expect("origin"))
            .expect("head")
            .expect("an entry");
        store
            .read(
                head.id,
                head.generation,
                RecordRange {
                    first: 1,
                    count: head.records,
                },
            )
            .expect("read")
    }

    fn segments(&self) -> usize {
        fs::read_dir(self.dir.join("store")).map_or(0, |entries| {
            entries
                .filter(|entry| {
                    entry
                        .as_ref()
                        .is_ok_and(|entry| entry.path().extension().is_some_and(|e| e == "seg"))
                })
                .count()
        })
    }

    fn again(&self) -> PathBuf {
        self.layout().state.join("store.again")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.main(), fs::Permissions::from_mode(0o600));
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A stamp within 2 seconds of a write is not kept, so the skip needs an older file.
fn settle(file: &Path) {
    fs::File::options()
        .write(true)
        .open(file)
        .expect("open the file")
        .set_modified(SystemTime::now() - Duration::from_secs(10))
        .expect("an older modification time");
}

fn lines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&byte| byte == b'\n').count()
}

#[test]
fn a_projects_folder_keeps_its_sessions_and_each_subagent_names_its_session_as_parent() {
    let sandbox = Sandbox::new("projects");
    let run = sandbox.run(&[text(&sandbox.projects()), "--kind", "session"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(3, 0, 0, 0, 0).as_str()),
        "the session and both subagents; nothing in memory/, tool-results/ or workflows/"
    );
    let main = sandbox.meta(&sandbox.main());
    let subagent = sandbox.meta(&sandbox.subagent());
    assert_eq!(main.kind, "session");
    assert_eq!(main.parent, None);
    assert_eq!(subagent.kind, "session");
    assert_eq!(subagent.parent, Some(main.id));
    assert_eq!(
        sandbox.meta(&sandbox.workflow_subagent()).parent,
        Some(main.id)
    );
}

#[test]
fn a_session_keeps_one_record_per_line_with_its_opaque_strings_as_placeholders() {
    let sandbox = Sandbox::new("records");
    assert_eq!(sandbox.run(&[text(&sandbox.rollout())]).code, 0);
    let records = sandbox.records(&sandbox.rollout());
    let originals: Vec<&[u8]> = CODEX.split_inclusive(|&byte| byte == b'\n').collect();
    assert_eq!(records.len(), originals.len());
    let changed: Vec<usize> = (0..records.len())
        .filter(|&at| records[at] != originals[at])
        .collect();
    assert_eq!(changed, [2, 3], "the image and the encrypted reasoning");
    for at in changed {
        let record = String::from_utf8(records[at].clone()).expect("UTF-8");
        assert!(record.contains("[opaque"), "{record}");
        assert!(record.ends_with('\n'));
    }
}

#[test]
fn a_grown_session_appends_its_new_lines_and_a_half_written_line_waits() {
    let sandbox = Sandbox::new("grown");
    let main = sandbox.main();
    assert_eq!(sandbox.run(&[text(&main)]).code, 0);
    let grown = [
        CLAUDE,
        b"{\"type\":\"user\",\"sessionId\":\"s\"}\n{\"type\":\"assi",
    ]
    .concat();
    write(&main, &grown);
    let run = sandbox.run(&[text(&main)]);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 1, 0, 0, 0).as_str())
    );
    assert_eq!(sandbox.records(&main).len(), lines(CLAUDE) + 1);
    let whole = [grown.as_slice(), b"stant\"}\n"].concat();
    write(&main, &whole);
    let run = sandbox.run(&[text(&main)]);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 1, 0, 0, 0).as_str()),
        "the finished line appends, it does not start a generation"
    );
    assert_eq!(sandbox.records(&main).len(), lines(CLAUDE) + 2);
}

#[test]
fn a_file_whose_size_and_time_did_not_change_is_not_read_again() {
    let sandbox = Sandbox::new("not_read");
    let main = sandbox.main();
    settle(&main);
    assert_eq!(sandbox.run(&[text(&main)]).code, 0);
    fs::set_permissions(&main, fs::Permissions::from_mode(0o000)).expect("chmod");
    let run = sandbox.run(&[text(&main)]);
    assert_eq!((run.code, run.stderr.clone()), (0, Vec::<String>::new()));
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 0, 0, 1, 0).as_str())
    );
}

#[test]
fn a_run_that_finds_the_again_flag_runs_the_follows_once_more_before_it_stops() {
    let sandbox = Sandbox::new("again");
    let notes = sandbox.dir.join("notes");
    write(&notes.join("a.md"), b"a\n");
    fs::create_dir_all(sandbox.layout().state).expect("state");
    fs::write(sandbox.again(), b"1").expect("the flag another bilbo set");
    let run = sandbox.run(&[text(&notes)]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(1, 0, 0, 0, 0).as_str())
    );
    assert_eq!(
        run.stderr,
        [
            "bilbo: ran the follows again for another bilbo: new 4, appended 0, new generation 0, unchanged 0, refused 0"
        ]
    );
    assert_eq!(sandbox.meta(&sandbox.rollout()).kind, "session");
    assert_eq!(fs::metadata(sandbox.again()).expect("flag").len(), 0);
}

fn indexed_to(sandbox: &Sandbox) -> u64 {
    let meta = fs::read_to_string(sandbox.layout().cache.join("index/meta.json"))
        .expect("the index's meta.json");
    let at = meta.find("\\nseq ").expect("a seq line") + "\\nseq ".len();
    meta[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("a number")
}

#[test]
fn a_bare_remember_keeps_what_a_followed_session_gained_and_indexes_it() {
    let sandbox = Sandbox::new("bare_grown");
    let first = sandbox.run(&[]);
    assert_eq!(first.code, 0, "{:?}", first.stderr);
    let seq = indexed_to(&sandbox);
    let more = b"{\"type\":\"user\",\"message\":{\"content\":\"one more\"}}\n";
    fs::write(sandbox.main(), [CLAUDE, more].concat()).expect("the session grows");
    let next = sandbox.run(&[]);
    assert_eq!((next.code, next.stderr.clone()), (0, Vec::<String>::new()));
    assert_eq!(
        next.stdout.lines().next(),
        Some(counts(0, 1, 0, 3, 0).as_str())
    );
    assert_eq!(sandbox.records(&sandbox.main()).len(), lines(CLAUDE) + 1);
    assert!(indexed_to(&sandbox) > seq, "the run indexed the turn");
}

#[expect(
    clippy::disallowed_methods,
    reason = "the done check is told its scratch store and the real folders by the environment"
)]
fn done_check_var(name: &str) -> OsString {
    std::env::var_os(name).unwrap_or_else(|| {
        panic!(
            "the done check needs BILBO_HOME, XDG_STATE_HOME, XDG_CACHE_HOME, BILBO_CONFIG, \
             SESSIONS_CLAUDE, SESSIONS_CODEX, READBACK_STARTED, READBACK_MANIFEST, READBACK_DIGESTS, \
             READBACK_DIGESTS_LATE and READBACK_HEADS; {name} is not set"
        )
    })
}

/// The files the kind must keep, found without its code: `*<uuid>.jsonl`, and `agent-*.jsonl` below
/// `<uuid>/subagents/`. Hidden entries, `memory/`, `workflows/` and `tool-results/` are skipped
/// outside `subagents/`. It lists plain files only: over a folder with packed rollouts the done
/// checks cover the plain ones.
fn session_files(
    folder: &Path,
    session: Option<&Path>,
    into: &mut Vec<(PathBuf, Option<PathBuf>)>,
) {
    let uuid = |text: &str| {
        text.len() == 36
            && text.bytes().enumerate().all(|(at, byte)| {
                matches!(at, 8 | 13 | 18 | 23) && byte == b'-' || byte.is_ascii_hexdigit()
            })
    };
    for entry in fs::read_dir(folder).expect("list a folder") {
        let entry = entry.expect("an entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().expect("a file type");
        let skipped = ["memory", "workflows", "tool-results"].contains(&name.as_str());
        if name.starts_with('.') || (session.is_none() && skipped) {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            let below = match session {
                Some(session) => Some(session.to_path_buf()),
                None if name == "subagents"
                    && folder
                        .file_name()
                        .is_some_and(|n| uuid(&n.to_string_lossy())) =>
                {
                    Some(folder.with_extension("jsonl"))
                }
                None => None,
            };
            session_files(&path, below.as_deref(), into);
        } else if kind.is_file() && name.ends_with(".jsonl") {
            let stem = &name[..name.len() - 6];
            let main = stem.len() >= 36
                && stem.is_char_boundary(stem.len() - 36)
                && uuid(&stem[stem.len() - 36..]);
            match session {
                Some(session) if name.starts_with("agent-") => {
                    into.push((path, Some(session.to_path_buf())));
                }
                _ if main => into.push((path, None)),
                _ => {}
            }
        }
    }
}

struct Listed {
    size: u64,
    modified: SystemTime,
    session: Option<PathBuf>,
}

/// `READBACK_MANIFEST`: `path<TAB>size<TAB>mtime_ns<TAB>session` per file, session `-` for a main
/// session.
fn manifest() -> BTreeMap<PathBuf, Listed> {
    let text = fs::read_to_string(done_check_var("READBACK_MANIFEST")).expect("the manifest");
    text.lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [path, size, nanos, session] = fields[..] else {
                panic!("a manifest line: {line}");
            };
            let listed = Listed {
                size: size.parse().expect("a size"),
                modified: SystemTime::UNIX_EPOCH
                    + Duration::from_nanos(nanos.parse().expect("mtime nanoseconds")),
                session: (session != "-").then(|| PathBuf::from(session)),
            };
            (PathBuf::from(path), listed)
        })
        .collect()
}

/// Settled: listed before the start and unchanged or gone since; it must then be kept whole.
/// Unreadable metadata counts as settled, so `read_back` fails on it.
fn is_settled(file: &Path, listed: &Listed, started: SystemTime) -> bool {
    listed.modified < started
        && fs::metadata(file).ok().is_none_or(|meta| {
            meta.len() == listed.size && meta.modified().is_ok_and(|time| time == listed.modified)
        })
}

/// Its mtime is before `READBACK_STARTED`, so the start walk should have listed it.
fn settled(file: &Path, started: SystemTime) -> bool {
    fs::metadata(file)
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| modified < started)
}

fn done_check_started() -> SystemTime {
    let millis: u64 = done_check_var("READBACK_STARTED")
        .to_str()
        .and_then(|text| text.parse().ok())
        .expect("READBACK_STARTED holds Unix milliseconds");
    SystemTime::UNIX_EPOCH + Duration::from_millis(millis)
}

fn done_check_store() -> Store {
    let vars: HashMap<String, OsString> = [
        "BILBO_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "BILBO_CONFIG",
        "HOME",
    ]
    .into_iter()
    .map(|name| (name.to_owned(), done_check_var(name)))
    .collect();
    let layout = settings::load(
        &|name| vars.get(name).cloned(),
        &|path| fs::read(path),
        &bilbo_cli::kind_keys(),
    )
    .expect("settings")
    .layout();
    Store::open_read_only(&layout, Options::default(), &mut |_| {}).expect("the store")
}

#[derive(Default)]
struct Counted {
    mains: usize,
    subagents: usize,
    unresolved: usize,
    records: usize,
    opaque: usize,
    hex: usize,
    grown: usize,
    gone: usize,
}

/// Kind `session`; a subagent's parent is its session's entry once kept; one record per complete
/// line, each the independent filter's form (`digests`), 40+ hex strings kept. A settled file is
/// whole, a live one may have grown; gone counts, any other read error fails.
#[expect(
    clippy::too_many_arguments,
    reason = "the read-back's state, shared by both passes"
)]
fn read_back(
    store: &Store,
    file: &Path,
    listed: &Listed,
    head: &Head,
    digests: Option<&Vec<String>>,
    settled: bool,
    counted: &mut Counted,
    problems: &mut Vec<String>,
) {
    let meta = store.meta(head.id).expect("meta").expect("a meta");
    if meta.kind != "session" {
        problems.push(format!("kind {}: {}", meta.kind, file.display()));
    }
    if let Some(main) = &listed.session {
        counted.subagents += 1;
        match head_of(store, main) {
            None => counted.unresolved += 1,
            Some(session) if meta.parent != Some(session.id) => {
                problems.push(format!("parent: {}", file.display()));
            }
            Some(_) => {}
        }
    } else {
        counted.mains += 1;
    }
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            counted.gone += 1;
            return;
        }
        Err(error) => {
            problems.push(format!(
                "cannot read a listed file: {}: {error}",
                file.display()
            ));
            return;
        }
    };
    let Some(digests) = digests else {
        problems.push(format!(
            "no digests for a file that exists: {}",
            file.display()
        ));
        return;
    };
    let Some(captured) = usize::try_from(head.mark.end)
        .ok()
        .and_then(|end| bytes.get(..end))
    else {
        problems.push(format!("shorter than what was kept: {}", file.display()));
        return;
    };
    let lines: Vec<&[u8]> = captured.split_inclusive(|&byte| byte == b'\n').collect();
    if captured.last().is_some_and(|&byte| byte != b'\n') {
        problems.push(format!("kept a line without its end: {}", file.display()));
    }
    let kept = store
        .read(
            head.id,
            head.generation,
            RecordRange {
                first: 1,
                count: head.records,
            },
        )
        .expect("read the records");
    if kept.len() != lines.len() {
        problems.push(format!(
            "{} records for {} complete lines up to byte {}: {}",
            kept.len(),
            lines.len(),
            head.mark.end,
            file.display()
        ));
        return;
    }
    if digests.len() < kept.len() {
        problems.push(format!("fewer lines than records: {}", file.display()));
        return;
    }
    if digests.len() > kept.len() {
        if settled {
            problems.push(format!(
                "kept {} of the {} lines of a settled file: {}",
                kept.len(),
                digests.len(),
                file.display()
            ));
        } else {
            counted.grown += 1;
        }
    }
    for (at, (record, line)) in kept.iter().zip(&lines).enumerate() {
        if fnv(record) != digests[at] {
            problems.push(format!(
                "record {} is not the line's stored form: {}",
                at + 1,
                file.display()
            ));
        } else if record.as_slice() != *line {
            counted.opaque += 1;
        }
        for run in hex_strings(line) {
            counted.hex += 1;
            if !record.windows(run.len()).any(|window| window == run) {
                problems.push(format!("a hex string was not kept: {}", file.display()));
            }
        }
    }
    counted.records += kept.len();
}

fn head_of(store: &Store, file: &Path) -> Option<Head> {
    store
        .by_origin(&Origin::file(file).expect("origin"))
        .expect("head")
}

#[test]
#[ignore = "reads the real session folders; the done check runs it against a scratch store"]
/// `SESSIONS_CLAUDE` and `SESSIONS_CODEX` (`:`-separated targets) are walked only to check the
/// start walk; `READBACK_DIGESTS` holds the filter's digests; `READBACK_HEADS` receives each file's
/// generation, records and end, or `-`.
#[expect(
    clippy::disallowed_macros,
    reason = "the done check prints what it checked"
)]
fn the_real_session_folders_read_back_whole() {
    let store = done_check_store();
    let started = done_check_started();
    let manifest = manifest();
    let expected = digests(&done_check_var("READBACK_DIGESTS"));
    let mut files = Vec::new();
    for variable in ["SESSIONS_CLAUDE", "SESSIONS_CODEX"] {
        let roots = done_check_var(variable);
        for root in roots.to_string_lossy().split(':').map(Path::new) {
            if root.is_dir() {
                session_files(root, None, &mut files);
            } else if root.exists() {
                files.push((root.to_path_buf(), None));
            }
        }
    }
    let mut problems = Vec::new();
    let mut new_since = 0;
    for (file, session) in &files {
        match manifest.get(file) {
            None if settled(file, started) => problems.push(format!(
                "the start walk lacks a settled file: {}",
                file.display()
            )),
            None => new_since += 1,
            Some(listed) if listed.session.as_ref() != session.as_ref() => {
                problems.push(format!(
                    "the walks disagree on its session: {}",
                    file.display()
                ));
            }
            Some(_) => {}
        }
    }
    let walked: BTreeSet<&PathBuf> = files.iter().map(|(file, _)| file).collect();
    let mut counted = Counted::default();
    let mut live_unkept = 0;
    let mut heads = String::new();
    for (file, listed) in &manifest {
        let settled = is_settled(file, listed, started);
        if settled && file.exists() && !walked.contains(file) {
            problems.push(format!("the walks disagree: {}", file.display()));
        }
        let Some(head) = head_of(&store, file) else {
            heads.push_str(&format!("{}\t-\n", file.display()));
            if settled {
                problems.push(format!("no entry: {}", file.display()));
            } else {
                live_unkept += 1;
            }
            continue;
        };
        heads.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            file.display(),
            head.generation.0,
            head.records,
            head.mark.end
        ));
        if head.generation.0 != 1 {
            problems.push(format!(
                "the first pass kept generation {}: {}",
                head.generation.0,
                file.display()
            ));
        }
        read_back(
            &store,
            file,
            listed,
            &head,
            expected.get(file),
            settled,
            &mut counted,
            &mut problems,
        );
    }
    fs::write(done_check_var("READBACK_HEADS"), heads).expect("write the heads");
    println!(
        "sessions {}, subagents {}, records {}, records with placeholders {}, \
         live and grown since kept {}, live and not kept yet {live_unkept}, gone since listed {}, \
         new since the start {new_since}, subagents whose session was not kept {}, \
         hex strings of 40 or more kept {}",
        counted.mains,
        counted.subagents,
        counted.records,
        counted.opaque,
        counted.grown,
        counted.gone,
        counted.unresolved,
        counted.hex
    );
    assert!(
        problems.is_empty(),
        "{} problems, first: {:?}",
        problems.len(),
        &problems[..problems.len().min(5)]
    );
}

#[test]
#[ignore = "reads the real session folders; the done check runs it against a scratch store"]
/// A file the first pass kept keeps its generation, a settled one its records and end too; one
/// first kept by the second pass is checked against `READBACK_DIGESTS_LATE`.
#[expect(
    clippy::disallowed_macros,
    reason = "the done check prints what it checked"
)]
fn the_real_session_folders_keep_their_heads_in_the_second_run() {
    let store = done_check_store();
    let started = done_check_started();
    let manifest = manifest();
    let late = digests(&done_check_var("READBACK_DIGESTS_LATE"));
    let heads = fs::read_to_string(done_check_var("READBACK_HEADS")).expect("the heads");
    let mut counted = Counted::default();
    let (mut checked, mut appended, mut kept_late, mut never_kept) = (0, 0, 0, 0);
    let mut rows = BTreeSet::new();
    let mut problems = Vec::new();
    for line in heads.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let file = Path::new(fields[0]);
        rows.insert(file.to_path_buf());
        let Some(listed) = manifest.get(file) else {
            problems.push(format!(
                "a heads row the manifest lacks: {}",
                file.display()
            ));
            continue;
        };
        let settled = is_settled(file, listed, started);
        checked += 1;
        let number = |text: &str| -> u64 { text.parse().expect("a number") };
        match (&fields[1..], head_of(&store, file)) {
            (["-"], None) if settled => problems.push(format!("no entry: {}", file.display())),
            (["-"], None) => never_kept += 1,
            (["-"], Some(head)) => {
                kept_late += 1;
                if head.generation.0 != 1 {
                    problems.push(format!(
                        "the second pass first kept it as generation {}: {}",
                        head.generation.0,
                        file.display()
                    ));
                }
                read_back(
                    &store,
                    file,
                    listed,
                    &head,
                    late.get(file),
                    settled,
                    &mut counted,
                    &mut problems,
                );
            }
            ([_, _, _], None) => problems.push(format!("lost its entry: {}", file.display())),
            ([generation, records, end], Some(head)) => {
                if u64::from(head.generation.0) != number(generation) {
                    problems.push(format!(
                        "a new generation of a listed file: {}",
                        file.display()
                    ));
                } else if head.records != number(records) || head.mark.end != number(end) {
                    if settled {
                        problems.push(format!(
                            "a settled file changed in the second run: {}",
                            file.display()
                        ));
                    } else {
                        appended += 1;
                    }
                }
            }
            _ => panic!("a heads line: {line}"),
        }
    }
    for file in manifest.keys() {
        if !rows.contains(file) {
            problems.push(format!(
                "a listed file has no heads row: {}",
                file.display()
            ));
        }
    }
    println!(
        "heads {checked}, live and appended in the second run {appended}, first kept in the second \
         run {kept_late}, live and never kept {never_kept}, gone since listed {}, of those first \
         kept late, subagents whose session was not kept {}",
        counted.gone, counted.unresolved
    );
    assert!(
        problems.is_empty(),
        "{} problems, first: {:?}",
        problems.len(),
        &problems[..problems.len().min(5)]
    );
}

/// `expected_stored.py --digests` output: path, one FNV-1a 64 digest per line's stored form, blank
/// line.
fn digests(path: &OsString) -> HashMap<PathBuf, Vec<String>> {
    let text = fs::read_to_string(path).expect("the digests");
    text.split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let mut lines = block.lines();
            let file = PathBuf::from(lines.next().expect("a path"));
            (file, lines.map(str::to_owned).collect())
        })
        .collect()
}

fn fnv(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// JSON strings of 40+ hex digits, quotes included: commit SHAs and digests.
fn hex_strings(line: &[u8]) -> Vec<&[u8]> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(start) = line[at..]
        .iter()
        .position(|&byte| byte == b'"')
        .map(|i| at + i)
    {
        let digits = line[start + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        let end = start + 1 + digits;
        if digits >= 40 && line.get(end) == Some(&b'"') {
            found.push(&line[start..=end]);
        }
        at = end.max(start + 1);
    }
    found
}

#[test]
fn a_subagent_kept_before_its_session_gets_its_parent_when_a_sweep_finds_it_grown() {
    let sandbox = Sandbox::new("child_first");
    let child = sandbox.subagent();
    settle(&child);
    assert_eq!(sandbox.run(&[text(&child)]).code, 0);
    assert_eq!(
        sandbox.meta(&child).parent,
        None,
        "its session is not kept yet"
    );
    let projects = text(&sandbox.projects()).to_owned();
    let run = sandbox.run(&[&projects, "--kind", "session"]);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(2, 0, 0, 1, 0).as_str())
    );
    assert_eq!(sandbox.meta(&child).parent, None, "not read, so it waits");
    write(
        &child,
        [SUBAGENT, b"{\"type\":\"user\",\"sessionId\":\"s\"}\n"].concat(),
    );
    let run = sandbox.run(&[&projects, "--kind", "session"]);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 1, 0, 2, 0).as_str())
    );
    assert_eq!(
        sandbox.meta(&child).parent,
        Some(sandbox.meta(&sandbox.main()).id)
    );
}

#[test]
fn a_bare_run_that_finds_only_a_reader_waits_for_it_and_keeps() {
    let sandbox = Sandbox::new("reader_held");
    assert_eq!(sandbox.run(&[text(&sandbox.rollout())]).code, 0);
    let reader = sandbox.store();
    let (notices_tx, notices) = channel();
    let run = thread::scope(|scope| {
        let bare = scope.spawn(|| sandbox.run_noticed(&[], Some(notices_tx)));
        let notice = notices.recv_timeout(WAIT).expect("the bare run waits");
        assert!(notice.contains("waiting for another bilbo"), "{notice}");
        assert_eq!(
            fs::metadata(sandbox.again()).map_or(0, |meta| meta.len()),
            0
        );
        drop(reader);
        bare.join().expect("the bare run")
    });
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(3, 0, 0, 1, 0).as_str())
    );
}

#[test]
fn a_follow_that_only_refuses_counts_its_refusals_once_and_exits_1() {
    let sandbox = Sandbox::new("follow_refused");
    let locked = sandbox.dir.join("locked");
    write(&locked.join("sub/a.md"), b"a\n");
    write(
        &sandbox.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
            text(&locked)
        )
        .as_bytes(),
    );
    fs::set_permissions(locked.join("sub"), fs::Permissions::from_mode(0o000)).expect("lock");
    let run = sandbox.run(&[]);
    fs::set_permissions(locked.join("sub"), fs::Permissions::from_mode(0o700)).expect("unlock");
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 0, 0, 0, 1).as_str())
    );
    let refusals = run
        .stderr
        .iter()
        .filter(|line| line.starts_with("bilbo: refused "))
        .count();
    assert_eq!(refusals, 1, "{:?}", run.stderr);
}

#[test]
fn a_rerun_with_a_follow_that_cannot_run_exits_1() {
    let sandbox = Sandbox::new("again_failed");
    let notes = sandbox.dir.join("notes");
    write(&notes.join("a.md"), b"a\n");
    write(
        &sandbox.dir.join("config/config.toml"),
        format!("[[follow]]\npath = \"{}\"\nkind = \"pdf\"\n", text(&notes)).as_bytes(),
    );
    fs::create_dir_all(sandbox.layout().state).expect("state");
    fs::write(sandbox.again(), b"1").expect("the flag another bilbo set");
    let run = sandbox.run(&[text(&notes)]);
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(1, 0, 0, 0, 0).as_str())
    );
    assert_eq!(
        run.stderr,
        [
            format!(
                "bilbo: follow {}: unknown kind 'pdf'; kinds: markdown, session",
                text(&notes)
            ),
            "bilbo: ran the follows again for another bilbo: new 0, appended 0, new generation 0, unchanged 0, refused 0".to_owned(),
        ]
    );
}

#[test]
fn two_follow_saves_at_once_keep_both_follows_and_the_files_own_bytes() {
    let sandbox = Sandbox::new("two_saves");
    let config = sandbox.dir.join("config/config.toml");
    write(&config, b"# mine\n[store]\ncompression = 3");
    let vars = sandbox.vars();
    let var = move |name: &str| vars.get(name).cloned();
    let settings =
        settings::load(&var, &|path| fs::read(path), &bilbo_cli::kind_keys()).expect("settings");
    let follow = |name: &str| Follow {
        path: sandbox.dir.join(name),
        kind: Some("markdown".to_owned()),
    };
    let (first, second) = (follow("one"), follow("two"));
    let (read_tx, read_rx) = channel();
    let (go_tx, go_rx) = channel::<()>();
    let (locking_tx, locking_rx) = channel();
    let (settings, var) = (&settings, &var);
    let (a, b) = thread::scope(|scope| {
        let first = &first;
        let a = scope.spawn(move || {
            save_with(settings, first, var, &mut |step| {
                if step == Saving::Read {
                    read_tx.send(()).ok();
                    go_rx.recv_timeout(WAIT).ok();
                }
            })
        });
        read_rx
            .recv_timeout(WAIT)
            .expect("the first save holds the lock and has read the file");
        let second = &second;
        let b = scope.spawn(move || {
            save_with(settings, second, var, &mut |step| {
                if step == Saving::Locking {
                    locking_tx.send(()).ok();
                }
            })
        });
        locking_rx
            .recv_timeout(WAIT)
            .expect("the second save reaches the lock while the first holds it");
        go_tx.send(()).expect("let the first write");
        (a.join().expect("first"), b.join().expect("second"))
    });
    assert_eq!(
        a,
        Ok(Saved::Added {
            config: config.clone(),
            defaults: true,
            unsynced: None
        })
    );
    assert_eq!(
        b,
        Ok(Saved::Added {
            config: config.clone(),
            defaults: false,
            unsynced: None
        }),
        "the second save read the file the first wrote"
    );
    let home = text(&sandbox.home()).to_owned();
    assert_eq!(
        fs::read_to_string(&config).expect("the config"),
        format!(
            "# mine\n[store]\ncompression = 3\n\n\
             [[follow]]\npath = \"{home}/.claude/projects\"\nkind = \"session\"\n\n\
             [[follow]]\npath = \"{home}/.codex/sessions\"\nkind = \"session\"\n\n\
             [[follow]]\npath = \"{}\"\nkind = \"markdown\"\n\n\
             [[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
            text(&first.path),
            text(&second.path)
        )
    );
}

#[test]
fn follow_refuses_a_config_its_user_cannot_write() {
    let sandbox = Sandbox::new("follow_unwritable");
    let config = sandbox.dir.join("config/config.toml");
    let notes = sandbox.dir.join("notes");
    write(&notes.join("a.md"), b"a\n");
    write(&config, b"# mine\n");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o422)).expect("chmod");
    let run = sandbox.run(&[text(&notes), "--follow"]);
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).expect("chmod back");
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(
        run.stderr[0],
        format!(
            "bilbo: cannot save the follow in {}: it is read-only; add these lines to it:",
            text(&config)
        )
    );
    assert_eq!(fs::read(&config).expect("the config"), b"# mine\n");
}

#[test]
fn follow_finds_a_folder_declared_through_a_link_followed_already() {
    let sandbox = Sandbox::new("follow_link");
    let config = sandbox.dir.join("config/config.toml");
    let notes = sandbox.dir.join("notes");
    let link = sandbox.dir.join("notes-link");
    write(&notes.join("a.md"), b"a\n");
    symlink(&notes, &link).expect("link");
    let declared = format!(
        "[[follow]]\npath = \"{}\"\nkind = \"markdown\"\n",
        text(&link)
    );
    write(&config, declared.as_bytes());
    let run = sandbox.run(&[text(&link), "--follow"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [format!("bilbo: {} is followed already", text(&notes))]
    );
    assert_eq!(fs::read(&config).expect("the config"), declared.as_bytes());
}

#[test]
fn a_bare_run_over_follows_that_claim_nothing_opens_and_closes_the_store() {
    let sandbox = Sandbox::new("bare_claims_nothing");
    let notes = sandbox.dir.join("notes");
    write(&notes.join("a.md"), b"a\n");
    assert_eq!(sandbox.run(&[text(&notes)]).code, 0);
    backdate(&sandbox.layout().state, 61);
    write(&sandbox.dir.join("config/config.toml"), b"follow = []\n");
    let run = sandbox.run(&[]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 0, 0, 0, 0).as_str())
    );
    assert!(run.notices.is_empty(), "{:?}", run.notices);
    assert_eq!(sandbox.segments(), 1);
    assert!(!sandbox.layout().state.join("open.since").exists());
}

#[test]
fn a_bare_run_whose_store_fails_still_says_what_it_planned() {
    let sandbox = Sandbox::new("bare_store_fails");
    let dir = text(&sandbox.dir).to_owned();
    write(&sandbox.dir.join("notes/a.md"), b"a\n");
    fs::create_dir_all(sandbox.dir.join("store/notes")).expect("a 0.20 root");
    write(
        &sandbox.dir.join("config/config.toml"),
        format!(
            "[[follow]]\npath = \"{dir}/gone\"\nkind = \"markdown\"\n\n\
             [[follow]]\npath = \"{dir}/notes\"\nkind = \"pdf\"\n"
        )
        .as_bytes(),
    );
    let run = sandbox.run(&[]);
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(run.stderr.len(), 3, "{:?}", run.stderr);
    assert_eq!(
        run.stderr[0],
        format!("bilbo: skipped {dir}/gone: no such file or folder")
    );
    assert_eq!(
        run.stderr[1],
        format!("bilbo: follow {dir}/notes: unknown kind 'pdf'; kinds: markdown, session")
    );
    assert!(
        run.stderr[2].starts_with(&format!("bilbo: {dir}/store/notes belongs to bilbo 0.20")),
        "{}",
        run.stderr[2]
    );
}

#[test]
fn follow_refuses_an_inline_follow_array_and_names_the_table_to_add() {
    for declared in [
        "follow = []\n",
        "# mine\nfollow = [{ path = \"/elsewhere\" }]\n\n[store]\ncompression = 3\n",
    ] {
        let sandbox = Sandbox::new("follow_inline");
        let config = sandbox.dir.join("config/config.toml");
        let notes = sandbox.dir.join("notes");
        write(&notes.join("a.md"), b"a\n");
        write(&config, declared.as_bytes());
        let run = sandbox.run(&[text(&notes), "--follow"]);
        assert_eq!(run.code, 2, "{declared:?}: {:?}", run.stderr);
        assert_eq!(
            run.stdout.lines().next(),
            Some(counts(1, 0, 0, 0, 0).as_str())
        );
        assert_eq!(
            run.stderr,
            [
                format!(
                    "bilbo: cannot save the follow in {}: it declares follow as an inline array; add this table to its follow array:",
                    text(&config)
                ),
                format!(
                    "bilbo: {{ path = \"{}\", kind = \"markdown\" }}",
                    text(&notes)
                ),
            ],
            "{declared:?}"
        );
        assert_eq!(fs::read(&config).expect("the config"), declared.as_bytes());
    }
}

fn head(bytes: &[u8], count: usize) -> Vec<u8> {
    bytes
        .split_inclusive(|&byte| byte == b'\n')
        .take(count)
        .flatten()
        .copied()
        .collect()
}

fn packed(bytes: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(bytes, 3).expect("packed")
}

#[test]
fn a_packed_rollout_is_kept_under_its_plain_path_and_reads_back_unpacked() {
    let plain = Sandbox::new("packed_reference");
    assert_eq!(plain.keep_sessions().code, 0);
    let expected = plain.records(&plain.rollout());
    let sandbox = Sandbox::new("packed_records");
    sandbox.leave(None, Some(CODEX_PACKED));
    let run = sandbox.keep_sessions();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(1, 0, 0, 0, 0).as_str())
    );
    assert_eq!(expected.len(), lines(CODEX));
    assert_eq!(sandbox.records(&sandbox.rollout()), expected);
}

#[test]
fn a_rollout_keeps_its_entry_however_often_it_changes_form() {
    let sandbox = Sandbox::new("forms");
    let kept = |plain: Option<&[u8]>, packed: Option<&[u8]>, expected: String| {
        sandbox.leave(plain, packed);
        let run = sandbox.keep_sessions();
        assert_eq!(run.code, 0, "{:?}", run.stderr);
        assert_eq!(run.stdout.lines().next(), Some(expected.as_str()));
    };
    let first_lines = head(CODEX, 3);
    kept(Some(&first_lines), None, counts(1, 0, 0, 0, 0));
    let id = sandbox.head(&sandbox.rollout()).id;
    kept(Some(CODEX), None, counts(0, 1, 0, 0, 0));
    kept(None, Some(CODEX_PACKED), counts(0, 0, 0, 1, 0));
    kept(Some(CODEX), None, counts(0, 0, 0, 1, 0));
    kept(None, Some(CODEX_PACKED), counts(0, 0, 0, 1, 0));
    kept(Some(CODEX), None, counts(0, 0, 0, 1, 0));
    let last = sandbox.head(&sandbox.rollout());
    assert_eq!((last.id, last.generation), (id, Generation(1)));
    assert_eq!(last.records, lines(CODEX) as u64);
}

#[test]
fn a_packed_rollout_that_holds_more_than_was_kept_appends() {
    let sandbox = Sandbox::new("packed_grown");
    sandbox.leave(Some(&head(CODEX, 3)), None);
    assert_eq!(sandbox.keep_sessions().code, 0);
    sandbox.leave(None, Some(CODEX_PACKED));
    let run = sandbox.keep_sessions();
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 1, 0, 0, 0).as_str()),
        "{:?}",
        run.stderr
    );
    assert_eq!(sandbox.records(&sandbox.rollout()).len(), lines(CODEX));
}

#[test]
fn a_plain_file_beside_its_packed_sibling_is_the_one_read() {
    let sandbox = Sandbox::new("both_forms");
    sandbox.leave(Some(&head(CODEX, 3)), None);
    assert_eq!(sandbox.keep_sessions().code, 0);
    sandbox.leave(Some(CODEX), Some(&packed(&head(CODEX, 3))));
    let run = sandbox.keep_sessions();
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 1, 0, 0, 0).as_str()),
        "{:?}",
        run.stderr
    );
    assert_eq!(sandbox.head(&sandbox.rollout()).generation, Generation(1));
}

#[test]
fn a_packed_file_that_does_not_unpack_is_refused_and_the_run_goes_on() {
    let sandbox = Sandbox::new("packed_cut");
    write(
        &sandbox.sessions().join("2026/11/07").join(SECOND),
        SECOND_BYTES,
    );
    sandbox.leave(None, Some(&CODEX_PACKED[..CODEX_PACKED.len() / 2]));
    let run = sandbox.keep_sessions();
    assert_eq!(run.code, 1, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(1, 0, 0, 0, 1).as_str())
    );
    let packed_name = format!("{}", sandbox.packed_rollout().display());
    assert!(
        run.stderr
            .iter()
            .any(|line| line.starts_with(&format!("bilbo: refused {packed_name}: cannot unpack"))),
        "{:?}",
        run.stderr
    );
    assert!(
        sandbox
            .store()
            .by_origin(&Origin::file(&sandbox.rollout()).expect("origin"))
            .expect("head")
            .is_none()
    );
}

#[test]
fn a_packed_file_made_to_hold_far_more_than_its_size_is_refused_on_every_run() {
    let sandbox = Sandbox::new("packed_bomb");
    write(
        &sandbox.sessions().join("2026/11/07").join(SECOND),
        SECOND_BYTES,
    );
    let bomb = packed(&[head(CODEX, 1), vec![b'\n'; 14 << 20]].concat());
    assert!(bomb.len() < 64 << 10, "{}", bomb.len());
    sandbox.leave(None, Some(&bomb));
    let refused = format!(
        "bilbo: refused {}: would hold more than 134217728 bytes unpacked",
        sandbox.packed_rollout().display()
    );
    for expected in [counts(1, 0, 0, 0, 1), counts(0, 0, 0, 1, 1)] {
        let run = sandbox.keep_sessions();
        assert_eq!(run.code, 1, "{:?}", run.stderr);
        assert_eq!(run.stdout.lines().next(), Some(expected.as_str()));
        assert!(run.stderr.contains(&refused), "{:?}", run.stderr);
    }
    assert!(
        sandbox
            .store()
            .by_origin(&Origin::file(&sandbox.rollout()).expect("origin"))
            .expect("head")
            .is_none()
    );
}

#[test]
fn the_unpack_keys_reach_the_session_kind() {
    let feeds = packed(&[head(CODEX, 1), vec![b'\n'; 1 << 20]].concat());
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let text_bytes: Vec<u8> = (0..256 << 10)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            b' ' + u8::try_from(state % 95).expect("a byte")
        })
        .collect();
    let hard = packed(&[head(SECOND_BYTES, 1), text_bytes, vec![b'\n'; 600_000]].concat());
    for (name, config, expected, code, refused) in [
        (
            "unpack_keys_default",
            "",
            counts(2, 0, 0, 0, 0),
            0,
            [false, false],
        ),
        (
            "unpack_keys_floor",
            "[session]\nunpack_floor = \"8 MiB\"\n",
            counts(1, 0, 0, 0, 1),
            1,
            [true, false],
        ),
        (
            "unpack_keys_floor_and_ratio",
            "[session]\nunpack_floor = \"8 MiB\"\nunpack_ratio = 64\n",
            counts(0, 0, 0, 0, 2),
            1,
            [true, true],
        ),
    ] {
        let sandbox = Sandbox::new(name);
        sandbox.leave(None, Some(&feeds));
        let mut second = sandbox
            .sessions()
            .join("2026/11/07")
            .join(SECOND)
            .into_os_string();
        second.push(".zst");
        write(Path::new(&second), &hard);
        write(&sandbox.dir.join("config/config.toml"), config.as_bytes());
        let run = sandbox.keep_sessions();
        assert_eq!(run.code, code, "{name}: {:?}", run.stderr);
        assert_eq!(run.stdout.lines().next(), Some(expected.as_str()), "{name}");
        for (file, refused) in [sandbox.packed_rollout(), PathBuf::from(second)]
            .iter()
            .zip(refused)
        {
            let line = format!("bilbo: refused {}: would hold more than ", file.display());
            assert_eq!(
                run.stderr.iter().any(|shown| shown.starts_with(&line)),
                refused,
                "{name}: {:?}",
                run.stderr
            );
        }
        if name == "unpack_keys_floor" {
            let line = format!(
                "bilbo: refused {}: would hold more than 8388608 bytes unpacked",
                sandbox.packed_rollout().display()
            );
            assert!(
                run.stderr.iter().any(|shown| shown == &line),
                "{:?}",
                run.stderr
            );
        }
    }
}

#[test]
fn a_link_under_a_rollouts_plain_name_is_never_read() {
    let reference = Sandbox::new("link_reference");
    assert_eq!(reference.keep_sessions().code, 0);
    let expected = reference.records(&reference.rollout());
    let sandbox = Sandbox::new("link");
    sandbox.leave(None, Some(CODEX_PACKED));
    let other = sandbox.dir.join("other.jsonl");
    write(&other, SECOND_BYTES);
    symlink(&other, sandbox.rollout()).expect("a link under the plain name");
    let run = sandbox.keep_sessions();
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(sandbox.records(&sandbox.rollout()), expected);
}

#[test]
fn a_followed_rollout_file_stays_followed_when_codex_packs_or_unpacks_it() {
    let sandbox = Sandbox::new("followed_forms");
    let followed = sandbox.dir.join("elsewhere").join(ROLLOUT);
    write(&followed, head(CODEX, 3));
    let run = sandbox.run(&[text(&followed), "--follow"]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    let run = sandbox.run(&[]);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(4, 0, 0, 1, 0).as_str()),
        "{:?}",
        run.stderr
    );
    let mut name = followed.clone().into_os_string();
    name.push(".zst");
    fs::remove_file(&followed).expect("Codex packs it");
    write(Path::new(&name), packed(&head(CODEX, 3)));
    let run = sandbox.run(&[]);
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 0, 0, 5, 0).as_str())
    );
    assert_eq!(run.stderr, Vec::<String>::new());
    fs::remove_file(Path::new(&name)).expect("Codex unpacks it");
    write(&followed, CODEX);
    let run = sandbox.run(&[]);
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 1, 0, 4, 0).as_str()),
        "{:?}",
        run.stderr
    );
}
