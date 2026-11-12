use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use bilbo_cli::console::settings;
use bilbo_cli::remember::save::{Saved, Saving, save_with};
use bilbo_kinds::follows::Follow;
use bilbo_store::entries::{Entries, Head, Meta, Origin, RecordRange};
use bilbo_store::layout::Layout;
use bilbo_store::segments::{Held, Options, Rebuild, Sealing, Store};

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
const CLAUDE_STOP: &str = include_str!("fixtures/hook/claude-code-stop.json");
const CODEX_STOP: &str = include_str!("fixtures/hook/codex-stop.json");

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

    fn vars(&self) -> HashMap<String, OsString> {
        vars(&self.dir)
    }

    fn layout(&self) -> Layout {
        layout(&self.vars())
    }

    fn run(&self, args: &[&str], stdin: &[u8]) -> Run {
        run(self.vars(), args, stdin, None)
    }

    fn run_noticed(&self, args: &[&str], stdin: &[u8], notices: Option<Sender<String>>) -> Run {
        run(self.vars(), args, stdin, notices)
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
    let run = sandbox.run(&[text(&sandbox.projects()), "--kind", "session"], b"");
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
    assert_eq!(sandbox.run(&[text(&sandbox.rollout())], b"").code, 0);
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
    assert_eq!(sandbox.run(&[text(&main)], b"").code, 0);
    let grown = [
        CLAUDE,
        b"{\"type\":\"user\",\"sessionId\":\"s\"}\n{\"type\":\"assi",
    ]
    .concat();
    write(&main, &grown);
    let run = sandbox.run(&[text(&main)], b"");
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(0, 1, 0, 0, 0).as_str())
    );
    assert_eq!(sandbox.records(&main).len(), lines(CLAUDE) + 1);
    let whole = [grown.as_slice(), b"stant\"}\n"].concat();
    write(&main, &whole);
    let run = sandbox.run(&[text(&main)], b"");
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
    assert_eq!(sandbox.run(&[text(&main)], b"").code, 0);
    fs::set_permissions(&main, fs::Permissions::from_mode(0o000)).expect("chmod");
    let run = sandbox.run(&[text(&main)], b"");
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
    let run = sandbox.run(&[text(&notes)], b"");
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

#[test]
fn a_hook_keeps_its_transcripts_new_lines_and_seals_nothing() {
    let sandbox = Sandbox::new("hook");
    let home = text(&sandbox.home()).to_owned();
    for payload in [CLAUDE_STOP, CODEX_STOP] {
        let payload = payload.replace("/home/me", &home);
        let run = sandbox.run(&["--hook"], payload.as_bytes());
        assert_eq!((run.code, run.stdout.as_str()), (0, ""));
        assert!(run.stderr.is_empty(), "{:?}", run.stderr);
        backdate(&sandbox.layout().state, 61);
    }
    assert_eq!(sandbox.records(&sandbox.main()).len(), lines(CLAUDE));
    assert_eq!(sandbox.records(&sandbox.rollout()).len(), lines(CODEX));
    assert_eq!(sandbox.segments(), 0, "the hook never seals, even when due");
    let open = fs::metadata(sandbox.layout().state.join("open.seg")).expect("open.seg");
    assert!(open.len() > 12);

    let run = sandbox.run(&[text(&sandbox.projects()), "--kind", "session"], b"");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        sandbox.segments(),
        1,
        "the next run with a target seals what is due"
    );
}

fn index_files(sandbox: &Sandbox) -> BTreeMap<String, Vec<u8>> {
    let folder = sandbox.layout().cache.join("index");
    fs::read_dir(folder)
        .expect("the index folder")
        .map(|entry| {
            let entry = entry.expect("an entry");
            (
                entry.file_name().into_string().expect("a name"),
                fs::read(entry.path()).unwrap_or_default(),
            )
        })
        .collect()
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
fn a_hook_keeps_its_turn_and_changes_no_file_of_the_cache_folders_index_and_the_next_remember_indexes_it()
 {
    let sandbox = Sandbox::new("hook_index");
    let first = sandbox.run(&[text(&sandbox.projects()), "--kind", "session"], b"");
    assert_eq!(first.code, 0, "{:?}", first.stderr);
    let files = index_files(&sandbox);
    let seq = indexed_to(&sandbox);
    let more = b"{\"type\":\"user\",\"message\":{\"content\":\"one more\"}}\n";
    fs::write(sandbox.main(), [CLAUDE, more].concat()).expect("the session grows");
    let payload = CLAUDE_STOP.replace("/home/me", text(&sandbox.home()));
    let hook = sandbox.run(&["--hook"], payload.as_bytes());
    assert_eq!((hook.code, hook.stdout.as_str()), (0, ""));
    assert!(hook.stderr.is_empty(), "{:?}", hook.stderr);
    assert_eq!(sandbox.records(&sandbox.main()).len(), lines(CLAUDE) + 1);
    assert_eq!(
        index_files(&sandbox),
        files,
        "the hook left the index alone"
    );
    let next = sandbox.run(&[text(&sandbox.projects()), "--kind", "session"], b"");
    assert_eq!((next.code, next.stderr.clone()), (0, Vec::<String>::new()));
    assert!(indexed_to(&sandbox) > seq, "the next run indexed the turn");
}

#[test]
fn a_bare_remember_that_finds_a_hook_holding_the_store_leaves_its_request_for_the_next_run_that_is_not_a_hook()
 {
    let sandbox = Sandbox::new("bare_finds_hook");
    let hook = Store::open(
        &sandbox.layout(),
        Options {
            held: Held::Flag {
                readers: Some(Duration::from_secs(1)),
            },
            sealing: Sealing::Never,
            rebuild: Rebuild::CatchUpOnly,
            ..Options::default()
        },
        &mut |_| {},
    )
    .expect("hold the store as a hook does");
    let run = sandbox.run(&[], b"");
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(
        run.stderr,
        ["bilbo: another bilbo is writing the store; requested another pass over the follows"]
    );
    hook.release();
    assert_eq!(fs::metadata(sandbox.again()).expect("flag").len(), 1);
    let next = sandbox.run(&[], b"");
    assert_eq!(next.code, 0, "{:?}", next.stderr);
    assert!(
        next.stderr
            .iter()
            .any(|line| line.starts_with("bilbo: ran the follows again for another bilbo: ")),
        "{:?}",
        next.stderr
    );
    assert_eq!(fs::metadata(sandbox.again()).expect("flag").len(), 0);
}

#[test]
fn a_hook_exits_0_whatever_it_is_given() {
    let sandbox = Sandbox::new("hook_garbage");
    let missing = CLAUDE_STOP.replace("/home/me", "/nowhere");
    for (input, line) in [
        (
            "garbage",
            "bilbo: the hook input is not a JSON object".to_owned(),
        ),
        (
            "{}",
            "bilbo: the hook input has no transcript_path".to_owned(),
        ),
        (
            missing.as_str(),
            format!(
                "bilbo: cannot read /nowhere/.claude/projects/-home-me-src-garden/{MAIN}.jsonl: No such file or directory (os error 2)"
            ),
        ),
    ] {
        let run = sandbox.run(&["--hook"], input.as_bytes());
        assert_eq!((run.code, run.stdout.as_str()), (0, ""), "{input}");
        assert_eq!(run.stderr, [line], "{input}");
    }
    write(
        &sandbox.dir.join("config/config.toml"),
        b"[store]\nseal_at = 0\n",
    );
    let run = sandbox.run(&["--hook"], CLAUDE_STOP.as_bytes());
    assert_eq!(run.code, 0);
    assert_eq!(run.stderr.len(), 1, "{:?}", run.stderr);
    assert!(!sandbox.dir.join("store").exists());
}

#[test]
fn a_hook_that_finds_the_store_held_sets_the_again_flag_and_exits_0() {
    let sandbox = Sandbox::new("hook_held");
    let layout = sandbox.layout();
    let held = Store::open(&layout, Options::default(), &mut |_| {}).expect("hold the store");
    let payload = CLAUDE_STOP.replace("/home/me", text(&sandbox.home()));
    let run = sandbox.run(&["--hook"], payload.as_bytes());
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert!(run.stderr.is_empty(), "{:?}", run.stderr);
    assert_eq!(fs::metadata(sandbox.again()).expect("flag").len(), 1);
    let bilbo_store::segments::Closed::Again(rerun) = held.close().expect("close") else {
        panic!("the holder must run again");
    };
    drop(rerun);
}

#[test]
fn a_hook_that_finds_a_reader_walking_the_segments_gives_up_within_its_wait_and_exits_0() {
    let sandbox = Sandbox::new("hook_reader");
    let first = sandbox.run(&[text(&sandbox.projects()), "--kind", "session"], b"");
    assert_eq!(first.code, 0, "{:?}", first.stderr);
    let kept = sandbox.records(&sandbox.main()).len();
    let more = b"{\"type\":\"user\",\"message\":{\"content\":\"one more\"}}\n";
    fs::write(sandbox.main(), [CLAUDE, more].concat()).expect("the session grows");
    fs::remove_file(sandbox.layout().cache.join("catalog.sqlite")).expect("delete the catalog");
    // With no catalog a reader walks every segment under the shared lock: it stands for a slow read.
    let reader = sandbox.store();
    let payload = CLAUDE_STOP.replace("/home/me", text(&sandbox.home()));
    for (wait, least) in [("0s", 0), ("1s", 1)] {
        let mut vars = sandbox.vars();
        vars.insert("BILBO_STORE_HOOK_WAIT".to_owned(), wait.into());
        let started = std::time::Instant::now();
        let hook = run(vars, &["--hook"], payload.as_bytes(), None);
        let took = started.elapsed().as_secs();
        assert_eq!((hook.code, hook.stdout.as_str()), (0, ""), "{wait}");
        assert_eq!(
            hook.stderr,
            [format!(
                "bilbo: kept nothing: another bilbo is reading the store; bilbo remember {} \
                     keeps these lines",
                text(&fs::canonicalize(sandbox.main()).expect("the transcript"))
            )],
            "{wait}"
        );
        assert!(
            (least..least + 3).contains(&took),
            "{wait}: it took {took} s"
        );
    }
    assert_eq!(
        fs::metadata(sandbox.again()).map_or(0, |flag| flag.len()),
        0,
        "a reader runs nothing again"
    );
    drop(reader);
    let bare = sandbox.run(&[], b"");
    assert_eq!(bare.code, 0, "{:?}", bare.stderr);
    assert_eq!(
        sandbox.records(&sandbox.main()).len(),
        kept + 1,
        "the next bare run keeps the line"
    );
}

#[test]
fn a_hook_that_gives_up_names_the_run_that_keeps_its_lines_whatever_folder_they_are_in() {
    let sandbox = Sandbox::new("hook_reader_named");
    let first = sandbox.run(&[text(&sandbox.projects()), "--kind", "session"], b"");
    assert_eq!(first.code, 0, "{:?}", first.stderr);
    fs::remove_file(sandbox.layout().cache.join("catalog.sqlite")).expect("delete the catalog");
    // Under the default follow, in a folder a bare run skips; and in a folder a shell would split.
    let hidden = sandbox.projects().join(format!(".hidden/{OTHER}.jsonl"));
    let spaced = sandbox.dir.join(format!("My Proj'ect/{OTHER}.jsonl"));
    write(&hidden, CLAUDE);
    write(&spaced, CLAUDE);
    let hidden = fs::canonicalize(&hidden).expect("the transcript");
    let spaced = fs::canonicalize(&spaced).expect("the transcript");
    let reader = sandbox.store();
    let mut commands = Vec::new();
    for file in [&hidden, &spaced] {
        let mut vars = sandbox.vars();
        vars.insert("BILBO_STORE_HOOK_WAIT".to_owned(), "0s".into());
        let payload = format!("{{\"transcript_path\":\"{}\"}}", text(file));
        let hook = run(vars, &["--hook"], payload.as_bytes(), None);
        assert_eq!((hook.code, hook.stdout.as_str()), (0, ""));
        let [line] = hook.stderr.as_slice() else {
            panic!("one line: {:?}", hook.stderr);
        };
        let command = line
            .strip_prefix("bilbo: kept nothing: another bilbo is reading the store; ")
            .and_then(|rest| rest.strip_suffix(" keeps these lines"))
            .unwrap_or_else(|| panic!("{line}"));
        commands.push(command.to_owned());
    }
    // A newline in a folder's name: no command could name the file on one line.
    let broken = sandbox.dir.join(format!("two\nlines/{OTHER}.jsonl"));
    write(&broken, CLAUDE);
    let broken = fs::canonicalize(&broken).expect("the transcript");
    let mut vars = sandbox.vars();
    vars.insert("BILBO_STORE_HOOK_WAIT".to_owned(), "0s".into());
    let payload = format!(
        "{{\"transcript_path\":\"{}\"}}",
        text(&broken).replace('\n', "\\n")
    );
    let hook = run(vars, &["--hook"], payload.as_bytes(), None);
    assert_eq!((hook.code, hook.stdout.as_str()), (0, ""));
    assert_eq!(
        hook.stderr,
        [
            "bilbo: kept nothing: another bilbo is reading the store, and the transcript's path \
             holds a control character, so no command for it can be printed: run bilbo remember \
             on that file"
        ]
    );
    assert_eq!(commands[0], format!("bilbo remember {}", text(&hidden)));
    assert_eq!(
        commands[1],
        format!("bilbo remember '{}'", text(&spaced).replace('\'', "'\\''"))
    );
    drop(reader);
    let bare = sandbox.run(&[], b"");
    assert_eq!(bare.code, 0, "{:?}", bare.stderr);
    for (file, command) in [&hidden, &spaced].into_iter().zip(&commands) {
        let origin = Origin::file(file).expect("origin");
        assert!(
            sandbox.store().by_origin(&origin).expect("head").is_none(),
            "a bare run does not come by {}",
            file.display()
        );
        let split = std::process::Command::new("/bin/sh")
            .args(["-c", &format!("set -- {command}; printf '%s\\n' \"$@\"")])
            .output()
            .expect("a shell");
        let split = String::from_utf8(split.stdout).expect("UTF-8");
        let words: Vec<&str> = split.lines().collect();
        assert_eq!(words, ["bilbo", "remember", text(file)]);
        let named = sandbox.run(&words[2..], b"");
        assert_eq!(named.code, 0, "{:?}", named.stderr);
        assert!(!sandbox.records(file).is_empty());
    }
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
/// outside `subagents/`.
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
fn a_hook_leaves_a_request_for_another_run_to_the_next_run_with_a_target() {
    let sandbox = Sandbox::new("hook_leaves_flag");
    let layout = sandbox.layout();
    fs::create_dir_all(&layout.state).expect("state");
    fs::write(sandbox.again(), b"1").expect("the flag another bilbo set");
    let payload = CLAUDE_STOP.replace("/home/me", text(&sandbox.home()));
    let run = sandbox.run(&["--hook"], payload.as_bytes());
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert!(run.stderr.is_empty(), "{:?}", run.stderr);
    assert_eq!(sandbox.records(&sandbox.main()).len(), lines(CLAUDE));
    assert_eq!(
        fs::metadata(sandbox.again()).expect("flag").len(),
        1,
        "the hook neither reran the follows nor took the request"
    );
    assert!(
        sandbox
            .store()
            .by_origin(&Origin::file(&sandbox.rollout()).expect("origin"))
            .expect("head")
            .is_none()
    );

    let run = sandbox.run(&[text(&sandbox.main())], b"");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_eq!(
        run.stderr,
        [
            "bilbo: ran the follows again for another bilbo: new 3, appended 0, new generation 0, unchanged 1, refused 0"
        ]
    );
    assert_eq!(fs::metadata(sandbox.again()).expect("flag").len(), 0);
}

#[test]
fn a_subagent_kept_before_its_session_gets_its_parent_when_a_sweep_finds_it_grown() {
    let sandbox = Sandbox::new("child_first");
    let child = sandbox.subagent();
    settle(&child);
    assert_eq!(sandbox.run(&[text(&child)], b"").code, 0);
    assert_eq!(
        sandbox.meta(&child).parent,
        None,
        "its session is not kept yet"
    );
    let projects = text(&sandbox.projects()).to_owned();
    let run = sandbox.run(&[&projects, "--kind", "session"], b"");
    assert_eq!(
        run.stdout.lines().next(),
        Some(counts(2, 0, 0, 1, 0).as_str())
    );
    assert_eq!(sandbox.meta(&child).parent, None, "not read, so it waits");
    write(
        &child,
        [SUBAGENT, b"{\"type\":\"user\",\"sessionId\":\"s\"}\n"].concat(),
    );
    let run = sandbox.run(&[&projects, "--kind", "session"], b"");
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
fn a_hook_that_finds_the_catalog_missing_or_stale_keeps_nothing_and_leaves_the_rebuild() {
    let sandbox = Sandbox::new("hook_unrebuilt");
    let layout = sandbox.layout();
    let catalog = layout.cache.join("catalog.sqlite");
    let open_seg = layout.state.join("open.seg");
    let payload = CLAUDE_STOP.replace("/home/me", text(&sandbox.home()));
    let refused = [
        "bilbo: kept nothing: the catalog must be rebuilt first, which the next bilbo remember without --hook does",
    ];
    assert_eq!(sandbox.run(&[text(&sandbox.rollout())], b"").code, 0);
    let one_segment_ago = fs::read(&catalog).expect("the catalog");

    fs::remove_file(&catalog).expect("lose the catalog");
    let before = fs::read(&open_seg).expect("open.seg");
    let started = Instant::now();
    let run = sandbox.run(&["--hook"], payload.as_bytes());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(run.stderr, refused);
    assert!(!catalog.exists(), "the hook rebuilt nothing");
    assert_eq!(
        fs::read(&open_seg).expect("open.seg"),
        before,
        "and kept nothing"
    );

    let run = sandbox.run(&[text(&sandbox.rollout())], b"");
    assert_eq!(run.code, 0, "{:?}", run.stderr);
    backdate(&layout.state, 61);
    assert_eq!(sandbox.run(&[text(&sandbox.rollout())], b"").code, 0);
    assert_eq!(sandbox.segments(), 1);
    fs::write(&catalog, &one_segment_ago).expect("a catalog older than the store");
    let before = fs::read(&open_seg).expect("open.seg");
    let run = sandbox.run(&["--hook"], payload.as_bytes());
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(run.stderr, refused);
    assert_eq!(fs::read(&catalog).expect("catalog"), one_segment_ago);
    assert_eq!(fs::read(&open_seg).expect("open.seg"), before);

    let run = sandbox.run(&["--hook"], payload.as_bytes());
    assert_eq!(run.stderr, refused, "still refused until a run rebuilds");
    assert_eq!(sandbox.run(&[text(&sandbox.rollout())], b"").code, 0);
    let run = sandbox.run(&["--hook"], payload.as_bytes());
    assert!(run.stderr.is_empty(), "{:?}", run.stderr);
    assert_eq!(sandbox.records(&sandbox.main()).len(), lines(CLAUDE));
}

#[test]
fn a_hook_whose_transcript_is_not_a_regular_file_keeps_nothing() {
    let sandbox = Sandbox::new("hook_not_a_file");
    let link = sandbox.dir.join("projects-link");
    symlink(sandbox.projects(), &link).expect("a link to a folder");
    for path in [sandbox.projects(), link, PathBuf::from("/dev/null")] {
        let payload = format!("{{\"transcript_path\":\"{}\"}}", text(&path));
        let run = sandbox.run(&["--hook"], payload.as_bytes());
        assert_eq!(
            (run.code, run.stdout.as_str()),
            (0, ""),
            "{}",
            path.display()
        );
        assert_eq!(
            run.stderr,
            [format!(
                "bilbo: the hook input's transcript_path is not a regular file: {}",
                text(&path)
            )]
        );
    }
    assert!(!sandbox.dir.join("store").exists(), "nothing kept");
}

#[test]
fn a_bare_run_that_finds_only_a_reader_waits_for_it_and_keeps() {
    let sandbox = Sandbox::new("reader_held");
    assert_eq!(sandbox.run(&[text(&sandbox.rollout())], b"").code, 0);
    let reader = sandbox.store();
    let (notices_tx, notices) = channel();
    let run = thread::scope(|scope| {
        let bare = scope.spawn(|| sandbox.run_noticed(&[], b"", Some(notices_tx)));
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
    let run = sandbox.run(&[], b"");
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
    let run = sandbox.run(&[text(&notes)], b"");
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
fn a_hook_never_seals_even_past_seal_at() {
    let sandbox = Sandbox::new("hook_past_seal_at");
    write(
        &sandbox.dir.join("config/config.toml"),
        b"[store]\nseal_at = \"1 MiB\"\n",
    );
    let transcript = sandbox
        .project()
        .join("0f1e2d3c-4b5a-4968-8776-a5b4c3d2e1f0.jsonl");
    write(&transcript, noisy_session(3000));
    let payload = format!("{{\"transcript_path\":\"{}\"}}", text(&transcript));
    let run = sandbox.run(&["--hook"], payload.as_bytes());
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert!(run.stderr.is_empty(), "{:?}", run.stderr);
    let open = fs::metadata(sandbox.layout().state.join("open.seg"))
        .expect("open.seg")
        .len();
    assert!(
        open > 1024 * 1024,
        "the capture took open.seg past seal_at: {open} bytes"
    );
    assert_eq!(sandbox.segments(), 0, "and the hook sealed nothing");
}

/// Lines of 1,000 pseudo-random characters, which zstd barely shrinks.
fn noisy_session(lines: usize) -> Vec<u8> {
    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789 ";
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut out = Vec::new();
    for n in 0..lines {
        let message: String = (0..1000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                char::from(alphabet[(state % 37) as usize])
            })
            .collect();
        out.extend_from_slice(
            format!(
                "{{\"type\":\"user\",\"sessionId\":\"s\",\"n\":{n},\"message\":\"{message}\"}}\n"
            )
            .as_bytes(),
        );
    }
    out
}

#[test]
fn follow_refuses_a_config_its_user_cannot_write() {
    let sandbox = Sandbox::new("follow_unwritable");
    let config = sandbox.dir.join("config/config.toml");
    let notes = sandbox.dir.join("notes");
    write(&notes.join("a.md"), b"a\n");
    write(&config, b"# mine\n");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o422)).expect("chmod");
    let run = sandbox.run(&[text(&notes), "--follow"], b"");
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
    let run = sandbox.run(&[text(&link), "--follow"], b"");
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
    assert_eq!(sandbox.run(&[text(&notes)], b"").code, 0);
    backdate(&sandbox.layout().state, 61);
    write(&sandbox.dir.join("config/config.toml"), b"follow = []\n");
    let run = sandbox.run(&[], b"");
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
    let run = sandbox.run(&[], b"");
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_eq!(run.stderr.len(), 3, "{:?}", run.stderr);
    assert_eq!(
        run.stderr[0],
        format!("bilbo: skipped {dir}/gone: no such folder")
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
        let run = sandbox.run(&[text(&notes), "--follow"], b"");
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
