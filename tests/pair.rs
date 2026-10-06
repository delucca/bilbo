//! `bilbo pair` through the built binary, on the golden fixtures of `tests/fixtures/device/`. No test has a terminal
//! and none waits on a mailbox, so this file reaches only the refusals and usage errors that come before one; the
//! whole exchange is a unit test of `identity::pair`, run in one process. Git keeps only the execute bit, so each
//! test copies the keys into a temporary folder and sets the modes.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Run, TempDir, bilbo, config};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");
const CODE: &str = "42-orbit-tunnel-velvet";

/// The three ways a run can be an agent's or a person's, each as the extra environment it sets.
const MARKS: [&[(&str, &str)]; 3] = [&[], &[("CLAUDECODE", "1")], &[("CODEX_THREAD_ID", "x")]];

struct Machine {
    dir: TempDir,
    config: PathBuf,
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Every file under `dir`, with its bytes, and every folder as an empty entry.
fn tree_of(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(dir: &Path, files: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.insert(path.clone(), None);
                walk(&path, files);
            } else {
                files.insert(path.clone(), Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(dir, &mut files);
    files
}

impl Machine {
    /// A machine with a store holding `notes/`, the keys of `who` (a fixture folder) when given, the fixture
    /// manifests when `manifests`, and a config of `lines`.
    fn new(name: &str, who: Option<&str>, manifests: bool, lines: &[&str]) -> Machine {
        let dir = TempDir::new(name);
        let config = config(&dir, lines);
        let machine = Machine { dir, config };
        fs::create_dir_all(machine.root().join("notes")).unwrap();
        if manifests {
            copy_tree(
                &Path::new(FIXTURES).join("store/.bilbo"),
                &machine.root().join(".bilbo"),
            );
        }
        if let Some(who) = who {
            let keys = machine.keys();
            copy_tree(&Path::new(FIXTURES).join(who), &keys);
            chmod(&keys, 0o700);
            for file in ["owner.key", "device.key"] {
                if keys.join(file).exists() {
                    chmod(&keys.join(file), 0o600);
                }
            }
        }
        machine
    }

    /// The enrolled `rivendell`, with the fixture manifests, whose `personal` scope syncs through `sync`.
    fn enrolled(name: &str, sync: &Path) -> Machine {
        Machine::new(name, Some("rivendell"), true, &[&personal(sync)])
    }

    /// A device with no keys and no manifests, holding a config of `lines`.
    fn blank(name: &str, lines: &[&str]) -> Machine {
        Machine::new(name, None, false, lines)
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("store")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state/bilbo")
    }

    fn keys(&self) -> PathBuf {
        self.state().join("keys")
    }

    /// A folder transport of its own, empty.
    fn sync(&self) -> PathBuf {
        let sync = self.dir.path().join("sync");
        fs::create_dir_all(&sync).unwrap();
        sync
    }

    fn pair(&self, args: &[&str]) -> Run {
        self.pair_with(&[], args)
    }

    /// Runs `bilbo pair <args>` with `extra` set beside the variables a machine needs.
    fn pair_with(&self, extra: &[(&str, &str)], args: &[&str]) -> Run {
        let root = self.root();
        let state = self.dir.path().join("state");
        let mut env = vec![
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", self.config.to_str().unwrap()),
            ("XDG_STATE_HOME", state.to_str().unwrap()),
            ("HOME", "/home/tester"),
        ];
        env.extend_from_slice(extra);
        let mut all = vec!["pair"];
        all.extend(args);
        bilbo(&std::env::temp_dir(), &env, &all)
    }

    /// Everything the machine holds apart from its config file, which a run may not touch either.
    fn tree(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        tree_of(self.dir.path())
    }
}

fn personal(sync: &Path) -> String {
    format!("scope.personal.sync = file://{}", sync.display())
}

fn url(sync: &Path) -> String {
    format!("file://{}", sync.display())
}

/// A refusal of exit `code`: nothing on stdout, a `bilbo: ` line holding `needle`.
fn refused(run: &Run, code: i32, needle: &str) {
    assert_eq!(run.code, code, "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains(needle), "{}", run.stderr);
}

/// A run that left the machine and its transport as they were, and made no mailbox.
struct Unchanged<'a> {
    machine: &'a Machine,
    sync: PathBuf,
    machine_before: BTreeMap<PathBuf, Option<Vec<u8>>>,
    sync_before: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

impl<'a> Unchanged<'a> {
    fn of(machine: &'a Machine, sync: &Path) -> Unchanged<'a> {
        Unchanged {
            machine,
            sync: sync.to_path_buf(),
            machine_before: machine.tree(),
            sync_before: tree_of(sync),
        }
    }

    fn check(&self) {
        assert!(!self.sync.join("pair").exists(), "a mailbox was made");
        assert_eq!(
            tree_of(&self.sync),
            self.sync_before,
            "the transport changed"
        );
        assert_eq!(
            self.machine.tree(),
            self.machine_before,
            "the machine changed"
        );
    }
}

// Show: refusals

#[test]
fn an_agent_shows_no_code_in_any_of_the_three_ways() {
    for (i, mark) in MARKS.iter().enumerate() {
        let m = Machine::enrolled(&format!("pair-agent-{i}"), Path::new("/placeholder"));
        let sync = m.sync();
        fs::write(&m.config, format!("{}\n", personal(&sync))).unwrap();
        let before = Unchanged::of(&m, &sync);
        let run = m.pair_with(mark, &[]);
        refused(
            &run,
            1,
            "pairing is confirmed only in a terminal, by the user",
        );
        before.check();
    }
}

#[test]
fn an_empty_agent_marker_is_still_no_terminal_without_a_tty() {
    let m = Machine::enrolled("pair-agent-empty", Path::new("/placeholder"));
    let sync = m.sync();
    fs::write(&m.config, format!("{}\n", personal(&sync))).unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair_with(&[("CLAUDECODE", ""), ("CODEX_THREAD_ID", "")], &[]);
    refused(
        &run,
        1,
        "pairing is confirmed only in a terminal, by the user",
    );
    before.check();
}

#[test]
fn a_device_with_no_owner_key_cannot_show_a_code() {
    let m = Machine::blank("pair-no-owner", &["scope.personal.sync = file:///srv/sync"]);
    let sync = m.sync();
    fs::write(&m.config, format!("{}\n", personal(&sync))).unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[]);
    refused(
        &run,
        1,
        "this device has no owner key; turn on sync for a scope first, which sets the recovery phrase",
    );
    before.check();
}

#[test]
fn no_syncing_scope_means_no_code() {
    let m = Machine::enrolled("pair-no-sync", Path::new("/placeholder"));
    fs::write(&m.config, "scope.personal.sync = off\n").unwrap();
    let sync = m.sync();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[]);
    refused(&run, 1, "no scope syncs");
    before.check();
}

#[test]
fn a_scope_that_does_not_sync_is_a_usage_error() {
    let m = Machine::enrolled("pair-off", Path::new("/placeholder"));
    let sync = m.sync();
    fs::write(
        &m.config,
        format!("{}\nscope.uber.sync = off\n", personal(&sync)),
    )
    .unwrap();
    let before = Unchanged::of(&m, &sync);
    refused(&m.pair(&["--scope", "uber"]), 2, "scope uber does not sync");
    refused(&m.pair(&["--scope", "nope"]), 2, "scope nope does not sync");
    before.check();
}

#[test]
fn scopes_in_two_folders_are_a_usage_error_naming_both_urls() {
    let m = Machine::enrolled("pair-two-folders", Path::new("/placeholder"));
    let sync = m.sync();
    fs::write(
        &m.config,
        "scope.personal.sync = file:///srv/a\nscope.shared.sync = file:///srv/b\n",
    )
    .unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[]);
    refused(&run, 2, "file:///srv/a");
    assert!(run.stderr.contains("file:///srv/b"), "{}", run.stderr);
    assert!(run.stderr.contains("--via"), "{}", run.stderr);
    before.check();
}

#[test]
fn the_same_folder_written_two_ways_is_two_urls() {
    let m = Machine::enrolled("pair-slash", Path::new("/placeholder"));
    let sync = m.sync();
    fs::write(
        &m.config,
        "scope.personal.sync = file:///srv/sync/\nscope.shared.sync = file:///srv/sync\n",
    )
    .unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[]);
    refused(&run, 2, "file:///srv/sync/");
    assert!(run.stderr.contains("file:///srv/sync,") || run.stderr.contains("file:///srv/sync)"));
    before.check();
}

#[test]
fn a_via_no_scope_uses_is_a_usage_error() {
    let m = Machine::enrolled("pair-via-unused", Path::new("/placeholder"));
    let sync = m.sync();
    fs::write(&m.config, format!("{}\n", personal(&sync))).unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&["--via", "file:///srv/other"]);
    refused(&run, 2, "no scope paired syncs through file:///srv/other");
    before.check();
}

#[test]
fn more_than_twelve_scopes_need_scope_flags() {
    let m = Machine::enrolled("pair-thirteen", Path::new("/placeholder"));
    let sync = m.sync();
    let lines: Vec<String> = (0..13)
        .map(|i| format!("scope.s{i}.sync = {}", url(&sync)))
        .collect();
    fs::write(&m.config, lines.join("\n") + "\n").unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[]);
    refused(&run, 2, "at most 12 scopes");
    assert!(run.stderr.contains("--scope"), "{}", run.stderr);
    before.check();
}

#[test]
fn a_relay_scope_is_checked_like_a_folder_scope() {
    let m = Machine::enrolled("pair-relay-show", Path::new("/placeholder"));
    let sync = m.sync();
    fs::write(
        &m.config,
        "scope.personal.sync = https://relay.example.net\n",
    )
    .unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[]);
    refused(&run, 1, "pairing is confirmed only in a terminal");
    before.check();
}

// Join: refusals before the mailbox

/// A new device whose transport folder holds an unrelated file, so a mailbox or a stray write shows.
fn joiner(name: &str) -> (Machine, PathBuf) {
    let m = Machine::blank(name, &["scope.personal.sync = off"]);
    let sync = m.sync();
    fs::write(sync.join("keep"), "held").unwrap();
    (m, sync)
}

#[test]
fn a_word_that_is_not_a_pairing_word_touches_nothing() {
    let (m, sync) = joiner("pair-bad-word");
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&["42-orbit-tunel-velvet", "--via", &url(&sync)]);
    refused(&run, 2, "'tunel' is not a pairing word");
    before.check();
}

#[test]
fn a_code_that_is_not_a_number_and_three_words_is_a_usage_error() {
    let (m, sync) = joiner("pair-bad-code");
    let before = Unchanged::of(&m, &sync);
    for code in [
        "orbit-tunnel-velvet",
        "42-orbit-tunnel",
        "0-orbit-tunnel-velvet",
        "1000-orbit-tunnel-velvet",
    ] {
        let run = m.pair(&[code, "--via", &url(&sync)]);
        assert_eq!(run.code, 2, "{code}: {}", run.stderr);
        assert!(run.stdout.is_empty(), "{}", run.stdout);
    }
    before.check();
}

#[test]
fn no_transport_given_is_a_usage_error() {
    let (m, sync) = joiner("pair-no-via");
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[CODE]);
    refused(&run, 2, "--via");
    before.check();
}

#[test]
fn a_remote_plain_http_url_is_a_usage_error_naming_it() {
    let (m, sync) = joiner("pair-http");
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[CODE, "--via", "http://bagend:8090"]);
    refused(&run, 2, "http://bagend:8090");
    before.check();
}

#[test]
fn a_relay_that_is_down_is_refused_naming_it() {
    let (m, sync) = joiner("pair-relay-join");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let via = format!("http://127.0.0.1:{port}");
    let run = m.pair(&[CODE, "--via", &via]);
    refused(&run, 1, &via);
    assert!(sync.join("keep").exists());
}

#[test]
fn a_missing_folder_is_refused() {
    let (m, sync) = joiner("pair-no-folder");
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[CODE, "--via", "file:///nope"]);
    refused(&run, 1, "no folder at /nope");
    before.check();
}

#[test]
fn no_store_is_refused() {
    let (m, sync) = joiner("pair-no-store");
    fs::remove_dir_all(m.root().join("notes")).unwrap();
    let before = Unchanged::of(&m, &sync);
    let run = m.pair(&[CODE, "--via", &url(&sync)]);
    refused(&run, 1, &format!("no store at {}", m.root().display()));
    before.check();
}

#[test]
fn a_refused_join_leaves_no_pending_key_or_state_folder() {
    let (m, sync) = joiner("pair-no-state");
    for via in ["file:///nope", "http://bagend:8090"] {
        m.pair(&[CODE, "--via", via]);
    }
    assert!(!m.state().exists(), "a refusal wrote the state folder");
    assert!(!m.keys().exists());
    assert!(sync.join("keep").exists());
}

// Usage errors

#[test]
fn stray_arguments_are_usage_errors() {
    let m = Machine::enrolled("pair-usage", Path::new("/placeholder"));
    let sync = m.sync();
    fs::write(&m.config, format!("{}\n", personal(&sync))).unwrap();
    let before = Unchanged::of(&m, &sync);
    for args in [
        &["--scope"][..],
        &["--via"],
        &["--frob"],
        &["--name", "bagend"],
        &[CODE, "--via", "file:///a", "--scope", "personal"],
        &[CODE, "--via", "file:///a", "--via", "file:///b"],
        &[CODE, "extra", "--via", "file:///a"],
    ] {
        let run = m.pair(args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty(), "{args:?}: {}", run.stdout);
        assert!(
            run.stderr.starts_with("bilbo: "),
            "{args:?}: {}",
            run.stderr
        );
    }
    before.check();
}

// Verb dispatch and config

#[test]
fn pair_is_a_verb() {
    let m = Machine::blank("pair-verb", &["scope.personal.sync = off"]);
    let run = m.pair(&[]);
    assert!(!run.stderr.contains("unknown verb"), "{}", run.stderr);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stderr.contains("no owner key"), "{}", run.stderr);
    let run = m.pair(&[CODE, "--via", "file:///nope"]);
    refused(&run, 1, "no folder at /nope");
}

#[test]
fn pair_reads_the_config() {
    let m = Machine::enrolled("pair-config", Path::new("/placeholder"));
    let sync = m.sync();
    let missing = m.dir.path().join("nope");
    let missing = missing.to_str().unwrap();
    let root = m.root();
    let state = m.dir.path().join("state");
    let env = [
        ("BILBO_HOME", root.to_str().unwrap()),
        ("BILBO_CONFIG", missing),
        ("XDG_STATE_HOME", state.to_str().unwrap()),
        ("HOME", "/home/tester"),
    ];
    let before = Unchanged::of(&m, &sync);
    let via = url(&sync);
    for args in [vec!["pair"], vec!["pair", CODE, "--via", via.as_str()]] {
        let run = bilbo(&std::env::temp_dir(), &env, &args);
        refused(&run, 2, missing);
    }
    before.check();
}
