//! `bilbo device` through the built binary, on the golden fixtures of `tests/fixtures/device/`: test keys (the
//! all-`abandon` owner and two devices with fixed seeds) and a store whose `personal` scope has versions 1 and 2.
//! Git keeps only the execute bit, so each test copies them into a temporary folder and sets the modes. No test has
//! a terminal, so the phrase forms are tested as refusals; the passing paths are unit tests of `identity::device`
//! and the smoke runs of `Terminal`. Regenerate the fixtures with
//! `cargo test --bin bilbo identity::device::tests::write_fixtures -- --ignored`.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Run, TempDir, bilbo, config};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/device");
const PERSONAL: &str = "scope.personal.sync = file:///Users/a/Sync/bilbo";
const FINGERPRINT: &str = "yb4b-5aju-v6zb-x2nm-nc5x-ompf";
const RIVENDELL: &str = "gr2q7gf5lh6pzfdnurnkvputhp";
const BAGEND: &str = "wyxim75c6m5p4ywv22ywilqweh";
const RELAY: &str = "scope.personal.sync = https://relay.example.net";

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

impl Machine {
    /// A machine holding the keys of `who` (a fixture folder) when given, the fixture store when `store`, and a
    /// config of `lines`.
    fn new(name: &str, who: Option<&str>, store: bool, lines: &[&str]) -> Machine {
        let dir = TempDir::new(name);
        let config = config(&dir, lines);
        let root = dir.path().join("store");
        fs::create_dir_all(&root).unwrap();
        if store {
            copy_tree(&Path::new(FIXTURES).join("store"), &root);
        }
        let machine = Machine { dir, config };
        if let Some(who) = who {
            let keys = machine.keys();
            copy_tree(&Path::new(FIXTURES).join(who), &keys);
            chmod(&keys, 0o700);
            for file in ["owner.key", "device.key"] {
                chmod(&keys.join(file), 0o600);
            }
        }
        machine
    }

    /// The enrolled `rivendell` with the fixture store and `personal` pinned to `file://`.
    fn enrolled(name: &str) -> Machine {
        Machine::new(name, Some("rivendell"), true, &[PERSONAL])
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

    fn scopes(&self) -> PathBuf {
        self.root().join(".bilbo/scopes")
    }

    /// The fixture scope's id: the one folder of the fixture store.
    fn scope(&self) -> String {
        let mut ids: Vec<String> = fs::read_dir(Path::new(FIXTURES).join("store/.bilbo/scopes"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(ids.len(), 1, "{ids:?}");
        ids.remove(0)
    }

    fn version(&self, n: u64) -> PathBuf {
        self.scopes()
            .join(self.scope())
            .join(format!("manifest/{n}.json"))
    }

    fn device(&self, args: &[&str]) -> Run {
        self.device_with(&[], args)
    }

    /// Runs `bilbo device <args>` with `extra` set beside the three variables a machine needs.
    fn device_with(&self, extra: &[(&str, &str)], args: &[&str]) -> Run {
        let root = self.root();
        let state = self.dir.path().join("state");
        let mut env = vec![
            ("BILBO_HOME", root.to_str().unwrap()),
            ("BILBO_CONFIG", self.config.to_str().unwrap()),
            ("XDG_STATE_HOME", state.to_str().unwrap()),
            ("HOME", "/home/tester"),
        ];
        env.extend_from_slice(extra);
        let mut all = vec!["device"];
        all.extend(args);
        bilbo(&std::env::temp_dir(), &env, &all)
    }

    /// Every file under the machine, with its bytes.
    fn tree(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, files);
                } else {
                    files.insert(path.clone(), fs::read(&path).unwrap());
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(self.dir.path(), &mut files);
        files
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

fn scope_line(id: &str) -> String {
    format!("scope\tpersonal\t{id}\tmanifest 2\tepoch 1\t2 devices\tfile://")
}

/// A refusal: exit 1, nothing on stdout, one `bilbo: ` line holding `needle`.
fn refused(run: &Run, needle: &str) {
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains(needle), "{}", run.stderr);
}

// Show

#[test]
fn show_prints_this_device_its_owner_and_the_scope() {
    let m = Machine::enrolled("device-show");
    let run = m.device(&[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run.stdout),
        [
            format!("device\trivendell\t{RIVENDELL}"),
            format!("owner\t{FINGERPRINT}"),
            scope_line(&m.scope()),
        ]
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

#[test]
fn the_other_device_shows_its_own_id_and_the_same_owner() {
    let m = Machine::new("device-show-bagend", Some("bagend"), true, &[PERSONAL]);
    let run = m.device(&[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run.stdout)[..2],
        [
            format!("device\tbagend\t{BAGEND}"),
            format!("owner\t{FINGERPRINT}")
        ]
    );
}

#[test]
fn a_device_with_no_keys_and_no_manifest_has_no_owner() {
    let m = Machine::new(
        "device-show-none",
        None,
        false,
        &["scope.personal.sync = off"],
    );
    let run = m.device(&[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "device\tnone\nowner\tnone\n");
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(!m.state().exists(), "show wrote the state folder");
}

#[test]
fn a_copied_store_with_no_keys_names_its_owner_and_the_way_in() {
    let m = Machine::new("device-show-copied", None, true, &[PERSONAL]);
    let run = m.device(&[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(&lines(&run.stdout)[..2], ["device\tnone", "owner\tnone"]);
    assert!(run.stdout.contains("\t2 devices\t"), "{}", run.stdout);
    assert!(run.stderr.contains(FINGERPRINT), "{}", run.stderr);
    assert!(
        run.stderr.contains("bilbo device recover"),
        "{}",
        run.stderr
    );
}

#[test]
fn an_extra_argument_is_a_usage_error() {
    let m = Machine::enrolled("device-show-extra");
    for args in [
        &["extra"][..],
        &["list", "extra"],
        &["--bogus"],
        &["init", "extra"],
    ] {
        let run = m.device(args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty());
    }
    let run = m.device(&["frobnicate"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("frobnicate"), "{}", run.stderr);
}

// List

#[test]
fn list_prints_every_device_and_marks_this_one() {
    let m = Machine::enrolled("device-list");
    let run = m.device(&["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run.stdout),
        [
            format!("bagend\t{BAGEND}"),
            format!("rivendell\t{RIVENDELL}\tthis")
        ]
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

#[test]
fn list_marks_the_other_device_on_the_other_machine() {
    let m = Machine::new("device-list-bagend", Some("bagend"), true, &[PERSONAL]);
    let run = m.device(&["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        lines(&run.stdout),
        [
            format!("bagend\t{BAGEND}\tthis"),
            format!("rivendell\t{RIVENDELL}")
        ]
    );
}

#[test]
fn list_without_keys_names_init_and_recover() {
    let m = Machine::new("device-list-none", None, true, &[PERSONAL]);
    let run = m.device(&["list"]);
    refused(&run, "bilbo device init");
    assert!(
        run.stderr.contains("bilbo device recover"),
        "{}",
        run.stderr
    );
}

// Init on an enrolled device

#[test]
fn a_rerun_of_init_keeps_everything_and_changes_no_file() {
    let m = Machine::enrolled("device-init-rerun");
    let before = m.tree();
    let run = m.device(&["init"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout.lines().count(), 3, "{}", run.stdout);
    assert!(
        run.stdout.lines().all(|l| l.contains(" kept")),
        "{}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    let mut after = m.tree();
    after.retain(|path, _| !path.ends_with(".bilbo/scopes/lock"));
    let mut before = before;
    before.retain(|path, _| !path.ends_with(".bilbo/scopes/lock"));
    assert_eq!(after, before);
}

#[test]
fn a_new_scope_is_sealed_without_a_terminal_and_lists_both_devices() {
    let m = Machine::new(
        "device-init-scope",
        Some("rivendell"),
        true,
        &[PERSONAL, "scope.shared.sync = file:///Users/a/Sync/shared"],
    );
    let run = m.device(&["init"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .lines()
            .any(|l| l.starts_with("scope shared created: ")),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .lines()
            .any(|l| l.starts_with("scope personal kept: ")),
        "{}",
        run.stdout
    );
    let shown = m.device(&[]);
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    let shared = shown
        .stdout
        .lines()
        .find(|l| l.starts_with("scope\tshared\t"))
        .unwrap();
    assert!(shared.contains("\tmanifest 1"), "{shared}");
    assert!(shared.contains("\t2 devices\t"), "{shared}");
    assert_eq!(
        shown
            .stdout
            .lines()
            .filter(|l| l.starts_with("scope\t"))
            .count(),
        2
    );
}

#[test]
fn changing_the_url_needs_a_terminal_and_leaves_the_manifest_alone() {
    let m = Machine::new(
        "device-init-url",
        Some("rivendell"),
        true,
        &[RELAY, "scope.shared.sync = file:///Users/a/Sync/shared"],
    );
    let two = fs::read(m.version(2)).unwrap();
    let run = m.device(&["init"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        run.stdout
            .lines()
            .any(|l| l == "scope personal failed: changing the URL needs a terminal"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .lines()
            .any(|l| l.starts_with("scope shared created: ")),
        "{}",
        run.stdout
    );
    assert!(!m.version(3).exists());
    assert_eq!(fs::read(m.version(2)).unwrap(), two);
}

#[test]
fn show_reports_a_url_the_manifest_does_not_pin() {
    let m = Machine::new("device-show-url", Some("rivendell"), true, &[RELAY]);
    let run = m.device(&[]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("file://"), "{}", run.stderr);
    assert!(
        run.stderr.contains("https://relay.example.net"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("bilbo device init"), "{}", run.stderr);
}

#[test]
fn two_configs_may_name_different_folders_for_one_file_scope() {
    let m = Machine::enrolled("device-folders");
    let first = m.device(&[]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    fs::write(
        &m.config,
        "scope.personal.sync = file:///home/b/elsewhere/bilbo\n",
    )
    .unwrap();
    let second = m.device(&[]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert!(second.stderr.is_empty(), "{}", second.stderr);
    assert_eq!(second.stdout, first.stdout);
}

// The terminal-only forms

#[test]
fn init_recover_and_revoke_need_a_terminal_without_keys() {
    let m = Machine::new("device-refuse-bare", None, false, &[]);
    let before = m.tree();
    for marks in MARKS {
        for args in [
            &["init", "--name", "rivendell"][..],
            &["recover", "--name", "rivendell"],
            &["revoke", "bagend"],
        ] {
            let run = m.device_with(marks, args);
            refused(&run, &format!("bilbo device {} needs a terminal", args[0]));
            assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
        }
    }
    assert_eq!(m.tree(), before);
    assert!(!m.state().exists());
}

#[test]
fn recover_and_revoke_need_a_terminal_on_an_enrolled_device() {
    let m = Machine::enrolled("device-refuse-enrolled");
    let before = m.tree();
    for marks in MARKS {
        for args in [&["recover"][..], &["revoke", "bagend"]] {
            let run = m.device_with(marks, args);
            refused(&run, &format!("bilbo device {} needs a terminal", args[0]));
        }
    }
    assert_eq!(m.tree(), before);
}

#[test]
fn the_refusal_comes_before_a_bad_name() {
    let m = Machine::new("device-refuse-name", None, false, &[]);
    let run = m.device(&["init", "--name", "Bad Name"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("--name"), "{}", run.stderr);
    for marks in MARKS {
        let run = m.device_with(marks, &["recover"]);
        refused(&run, "needs a terminal");
    }
}

#[test]
fn revoke_needs_an_argument() {
    let m = Machine::enrolled("device-revoke-arg");
    let run = m.device(&["revoke"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("revoke needs a device"),
        "{}",
        run.stderr
    );
    let run = m.device(&["revoke", "a", "b"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
}

// Keys on disk

#[test]
fn loose_modes_refuse_every_form_and_name_the_path() {
    let m = Machine::enrolled("device-modes");
    chmod(&m.keys(), 0o755);
    for args in [&[][..], &["list"], &["init"]] {
        let run = m.device(args);
        refused(&run, &m.keys().display().to_string());
    }
    chmod(&m.keys(), 0o700);
    let file = m.keys().join("owner.key");
    chmod(&file, 0o644);
    let run = m.device(&[]);
    refused(&run, &file.display().to_string());
    chmod(&file, 0o600);
    assert_eq!(m.device(&[]).code, 0);
}

#[test]
fn a_leftover_keys_new_is_named_and_left_alone_by_show_and_list() {
    let m = Machine::enrolled("device-leftover");
    let left = m.state().join("keys.new");
    fs::create_dir_all(&left).unwrap();
    fs::write(left.join("owner.key"), "x").unwrap();
    for args in [&[][..], &["list"]] {
        let run = m.device(args);
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert_eq!(
            run.stderr
                .lines()
                .filter(|l| l.contains("keys.new"))
                .count(),
            1,
            "{}",
            run.stderr
        );
        assert!(left.join("owner.key").is_file());
    }
}

#[test]
fn a_leftover_keys_new_without_keys_is_named_too() {
    let m = Machine::new("device-leftover-bare", None, false, &[]);
    let left = m.state().join("keys.new");
    fs::create_dir_all(&left).unwrap();
    let run = m.device(&[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "device\tnone\nowner\tnone\n");
    assert!(run.stderr.contains("keys.new"), "{}", run.stderr);
    assert!(left.is_dir());
}

#[test]
fn a_damaged_key_file_is_refused() {
    let m = Machine::enrolled("device-damaged");
    fs::write(m.keys().join("device.key"), "{\"format\":1,\"nope\":").unwrap();
    for args in [&[][..], &["list"]] {
        let run = m.device(args);
        assert_eq!(run.code, 1, "{}", run.stderr);
        assert!(run.stdout.is_empty(), "{}", run.stdout);
        assert!(run.stderr.contains("device.key"), "{}", run.stderr);
    }
}

// Manifests that fail

#[test]
fn a_tampered_manifest_is_a_problem_and_exits_1() {
    let m = Machine::enrolled("device-tampered");
    let path = m.version(2);
    let mut bytes = fs::read(&path).unwrap();
    let at = bytes.windows(7).position(|b| b == b"\"name\":").unwrap();
    bytes[at + 10] ^= 1;
    fs::write(&path, &bytes).unwrap();
    let run = m.device(&[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        run.stderr.contains("manifest/") && run.stderr.contains("invalid"),
        "{}",
        run.stderr
    );
    assert!(run.stdout.contains(&m.scope()), "{}", run.stdout);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn a_reformatted_manifest_is_not_canonical() {
    let m = Machine::enrolled("device-reformatted");
    let path = m.version(2);
    let text = fs::read_to_string(&path).unwrap();
    let spaced = text.replacen("{\"format\":1,", "{ \"format\": 1,", 1);
    assert_ne!(spaced, text);
    fs::write(&path, &spaced).unwrap();
    let run = m.device(&[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        run.stderr.contains("manifest/") && run.stderr.contains("invalid"),
        "{}",
        run.stderr
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), spaced);
}

#[test]
fn a_manifest_in_another_scopes_folder_is_a_problem() {
    let m = Machine::enrolled("device-misplaced");
    let id = m.scope();
    let moved = "a".repeat(id.len());
    copy_tree(&m.scopes().join(&id), &m.scopes().join(&moved));
    let run = m.device(&[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stderr.contains(&moved), "{}", run.stderr);
    assert!(
        run.stdout
            .lines()
            .any(|l| l.starts_with(&format!("scope\t-\t{moved}\t"))),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains(&scope_line(&id)), "{}", run.stdout);
}

#[test]
fn a_manifest_of_another_owner_is_a_problem_naming_both() {
    let m = Machine::enrolled("device-foreign");
    copy_tree(
        &Path::new(FIXTURES).join("foreign/.bilbo/scopes"),
        &m.scopes(),
    );
    let run = m.device(&[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let line = run
        .stderr
        .lines()
        .find(|l| l.contains("is signed by owner"))
        .unwrap_or_else(|| panic!("{}", run.stderr));
    assert!(line.contains(FINGERPRINT), "{line}");
    assert!(
        run.stdout.contains(&scope_line(&m.scope())),
        "{}",
        run.stdout
    );
}

#[test]
fn revoke_refuses_a_tampered_scope_without_a_terminal_first() {
    let m = Machine::enrolled("device-tampered-revoke");
    fs::write(m.version(2), "{}\n").unwrap();
    let before = m.tree();
    let run = m.device(&["revoke", "bagend"]);
    refused(&run, "needs a terminal");
    assert_eq!(m.tree(), before);
}

// Config scenarios that name `bilbo device`

#[test]
fn an_explicit_config_that_does_not_exist_is_exit_2() {
    let m = Machine::enrolled("device-config-missing");
    let root = m.root();
    let state = m.dir.path().join("state");
    for args in [&[][..], &["list"]] {
        let mut all = vec!["device"];
        all.extend(args);
        let run = bilbo(
            &std::env::temp_dir(),
            &[
                ("BILBO_HOME", root.to_str().unwrap()),
                ("BILBO_CONFIG", "/tmp/nope"),
                ("XDG_STATE_HOME", state.to_str().unwrap()),
                ("HOME", "/home/tester"),
            ],
            &all,
        );
        assert_eq!(run.code, 2, "{}", run.stderr);
        assert!(run.stdout.is_empty());
        assert!(run.stderr.contains("/tmp/nope"), "{}", run.stderr);
    }
}

#[test]
fn plain_http_to_another_host_is_exit_2() {
    let m = Machine::new(
        "device-config-http",
        Some("rivendell"),
        true,
        &["scope.personal.sync = http://relay.example.net"],
    );
    let run = m.device(&[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("scope.personal.sync"), "{}", run.stderr);
}

#[test]
fn a_password_in_the_url_is_not_echoed() {
    let m = Machine::new(
        "device-config-secret",
        Some("rivendell"),
        true,
        &["scope.personal.sync = https://u:sekrit@relay.example.net"],
    );
    for args in [&[][..], &["list"], &["init"]] {
        let run = m.device(args);
        assert_eq!(run.code, 2, "{}", run.stderr);
        assert!(run.stderr.contains("scope.personal.sync"), "{}", run.stderr);
        assert!(!run.stderr.contains("sekrit"), "{}", run.stderr);
        assert!(!run.stdout.contains("sekrit"), "{}", run.stdout);
    }
}
