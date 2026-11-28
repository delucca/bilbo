use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use bilbo_setup::program::{self, Bounds, End};
use bilbo_setup::talk::{Calls, Fault, Talk};
use serde_json::json;

mod timers;
mod tools;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
static NEXT: AtomicU32 = AtomicU32::new(0);

struct Folder(PathBuf);

impl std::ops::Deref for Folder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch() -> Folder {
    loop {
        let dir = std::env::temp_dir().join(format!(
            "bilbo-setup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return Folder(dir),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("{}: {error}", dir.display()),
        }
    }
}

fn exec_bit(path: &Path, on: bool) {
    let mode = if on { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("a mode");
}

fn script(folder: &Path, name: &str, body: &str) -> PathBuf {
    let path = folder.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("a script");
    exec_bit(&path, true);
    path
}

fn bounds(wait: Duration) -> Bounds {
    Bounds {
        wait,
        poll: Duration::from_millis(10),
    }
}

const SECOND: Duration = Duration::from_secs(1);
const LIMIT: Duration = Duration::from_secs(15);

fn serial() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn a_program_is_its_path_or_the_first_on_path() {
    let _serial = serial();
    let (first, second, third) = (scratch(), scratch(), scratch());
    fs::create_dir(first.join("tool")).expect("a folder named tool");
    fs::write(second.join("tool"), "#!/bin/sh\n").expect("a file");
    exec_bit(&second.join("tool"), false);
    let found = script(&third, "tool", "exit 0");
    let path = format!(
        "{}:{}:{}",
        first.display(),
        second.display(),
        third.display()
    );
    assert_eq!(program::find("tool", &path), Some(found.clone()));
    let other = scratch();
    script(&other, "tool", "exit 0");
    let both = format!("{}:{}", third.display(), other.display());
    assert_eq!(program::find("tool", &both), Some(found.clone()));
    assert_eq!(program::find("missing", &path), None);

    let cwd = std::env::current_dir().expect("a working folder");
    let up = "../".repeat(
        cwd.components()
            .filter(|part| matches!(part, Component::Normal(_)))
            .count(),
    );
    let relative = format!(
        "{up}{}",
        third.strip_prefix("/").expect("absolute").display()
    );
    assert!(Path::new(&relative).join("tool").is_file());
    assert_eq!(program::find("tool", &relative), None);
    assert_eq!(
        program::find("tool", format!("{relative}:{}", third.display())),
        Some(found.clone())
    );

    let bytes = scratch();
    let raw = bytes.join(OsStr::from_bytes(b"d\xff"));
    if fs::create_dir(&raw).is_ok() {
        let lossy = bytes.join("d\u{fffd}");
        fs::create_dir(&lossy).expect("a folder named as the text reads");
        let first = script(&raw, "tool", "exit 0");
        script(&lossy, "tool", "exit 0");
        assert_eq!(
            program::find("tool", raw.as_os_str()),
            Some(first),
            "a PATH that is no UTF-8 is looked through as the bytes it is"
        );
    }

    let odd = scratch();
    let owned = odd.join("tool");
    fs::write(&owned, "#!/bin/sh\nexit 0\n").expect("a script");
    fs::set_permissions(&owned, fs::Permissions::from_mode(0o601)).expect("a mode");
    let starts = program::run(&owned, &[], bounds(LIMIT)).end == End::Exited(0);
    let owned_path = owned.to_str().expect("UTF-8");
    assert_eq!(program::find(owned_path, "").is_some(), starts);
    let ahead = format!("{}:{}", odd.display(), third.display());
    let expected = if starts { owned.clone() } else { found.clone() };
    assert_eq!(program::find("tool", &ahead), Some(expected));

    let given = found.to_str().expect("UTF-8");
    assert_eq!(program::find(given, ""), Some(found));
    let plain = second.join("tool");
    assert_eq!(program::find(plain.to_str().expect("UTF-8"), ""), None);
}

#[test]
fn a_run_gives_the_exit_code_and_both_streams() {
    let _serial = serial();
    let folder = scratch();
    let tool = script(&folder, "tool", "echo out\necho err >&2\nexit 3");
    let ran = program::run(&tool, &[], bounds(LIMIT));
    assert_eq!(ran.end, End::Exited(3));
    assert_eq!(ran.stdout, b"out\n");
    assert_eq!(ran.stderr, b"err\n");
}

#[test]
fn a_run_that_outlives_the_wait_is_stopped_and_said_so() {
    let _serial = serial();
    let folder = scratch();
    let tool = script(&folder, "tool", "sleep 30");
    let ran = program::run(&tool, &[], bounds(SECOND));
    assert_eq!(ran.end, End::StillRunning);
}

#[test]
fn a_run_whose_child_keeps_its_output_open_ends_at_the_wait() {
    let _serial = serial();
    let folder = scratch();
    let tool = script(&folder, "tool", "echo half\nsleep 30 &\nexit 0");
    let ran = program::run(&tool, &[], bounds(SECOND));
    assert_eq!(ran.end, End::OutputOpen);
    assert!(ran.stdout.is_empty());
}

fn fifo(folder: &Path) -> PathBuf {
    let path = folder.join("held.fifo");
    let made = Command::new("mkfifo").arg(&path).status().expect("mkfifo");
    assert!(made.success());
    path
}

fn watching(fifo: &Path) -> mpsc::Receiver<io::Result<usize>> {
    let (sender, receiver) = mpsc::channel();
    let path = fifo.to_owned();
    thread::spawn(move || {
        let read = File::open(&path).and_then(|mut file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
        });
        let _ = sender.send(read);
    });
    receiver
}

fn ended(watch: &mpsc::Receiver<io::Result<usize>>) {
    let read = watch
        .recv_timeout(LIMIT)
        .expect("every process that held the pipe has stopped");
    assert_eq!(read.expect("a read"), 0);
}

#[test]
fn a_process_a_run_left_behind_has_stopped_after_the_wait() {
    let _serial = serial();
    for body in [
        "sleep 30 3>\"$1\" &\nexit 0",
        "sleep 30 3>\"$1\" &\nexec 3>\"$1\"\nexec sleep 30",
    ] {
        let folder = scratch();
        let tool = script(&folder, "tool", body);
        let held = fifo(&folder);
        let watch = watching(&held);
        let ran = program::run(&tool, &[held.to_str().expect("UTF-8")], bounds(SECOND));
        assert!(
            matches!(ran.end, End::OutputOpen | End::StillRunning),
            "{body}"
        );
        ended(&watch);
    }
}

#[test]
fn a_run_that_prints_more_than_a_pipe_holds_is_read_whole() {
    let _serial = serial();
    let folder = scratch();
    let tool = script(
        &folder,
        "tool",
        "head -c 2097152 /dev/zero\nhead -c 2097152 /dev/zero >&2",
    );
    let ran = program::run(&tool, &[], bounds(LIMIT));
    assert_eq!(ran.end, End::Exited(0));
    assert_eq!((ran.stdout.len(), ran.stderr.len()), (2_097_152, 2_097_152));
}

#[test]
fn a_program_that_cannot_start_is_said_with_the_systems_error() {
    let _serial = serial();
    let folder = scratch();
    let ran = program::run(&folder.join("nothing"), &[], bounds(SECOND));
    let End::NotStarted(error) = ran.end else {
        panic!("{:?}", ran.end);
    };
    assert!(error.contains("No such file"), "{error}");
    assert!(Talk::start(&folder.join("nothing"), &[], bounds(SECOND)).is_err());
}

fn peer(folder: &Path, body: &str) -> Talk {
    let tool = script(folder, "peer", body);
    Talk::start(&tool, &[], bounds(SECOND)).expect("a peer starts")
}

fn peer_with(folder: &Path, body: &str, args: &[&str]) -> Talk {
    let tool = script(folder, "peer", body);
    Talk::start(&tool, args, bounds(SECOND)).expect("a peer starts")
}

fn holding(folder: &Path, body: &str) -> (Talk, mpsc::Receiver<io::Result<usize>>) {
    let held = fifo(folder);
    let watch = watching(&held);
    let talk = peer_with(
        folder,
        &format!("exec 3>\"$1\"\n{body}"),
        &[held.to_str().expect("UTF-8")],
    );
    (talk, watch)
}

#[test]
fn replies_are_matched_by_id_and_other_lines_passed_over() {
    let _serial = serial();
    let folder = scratch();
    let mut talk = peer(
        &folder,
        "read a\necho '{\"method\":\"note\",\"params\":{}}'\necho 'not json'\n\
         echo '{\"id\":99,\"result\":1}'\necho '{\"id\":1,\"method\":\"asks\"}'\n\
         echo '{\"id\":1,\"result\":{\"ok\":true}}'",
    );
    assert_eq!(talk.request("m", json!({})), Ok(json!({"ok": true})));
    talk.close();
}

#[test]
fn requests_count_from_one_and_a_notice_has_no_id() {
    let _serial = serial();
    let folder = scratch();
    let odd = folder.join("it's here");
    fs::create_dir(&odd).expect("a folder with an apostrophe");
    let log = odd.join("log");
    let mut talk = peer_with(
        &folder,
        "log=$1\nread a\necho \"$a\" >> \"$log\"\necho '{\"id\":1,\"result\":1}'\n\
         read b\necho \"$b\" >> \"$log\"\nread c\necho \"$c\" >> \"$log\"\n\
         read d\necho \"$d\" >> \"$log\"\necho '{\"id\":2,\"result\":2}'",
        &[log.to_str().expect("UTF-8")],
    );
    assert_eq!(talk.request("a", json!({})), Ok(json!(1)));
    assert_eq!(talk.notice("b", Some(json!({}))), Ok(()));
    assert_eq!(talk.notice("c", None), Ok(()));
    assert_eq!(talk.request("d", json!({})), Ok(json!(2)));
    talk.close();
    assert_eq!(
        fs::read_to_string(&log).expect("what the peer read"),
        "{\"id\":1,\"method\":\"a\",\"params\":{}}\n{\"method\":\"b\",\"params\":{}}\n\
         {\"method\":\"c\"}\n{\"id\":2,\"method\":\"d\",\"params\":{}}\n"
    );
}

#[test]
fn a_peer_that_does_not_answer_within_the_wait_is_stopped() {
    let _serial = serial();
    let folder = scratch();
    let (mut talk, watch) = holding(&folder, "read a\nsleep 30");
    assert_eq!(
        talk.request("m", json!({})),
        Err(Fault::Silent {
            method: "m".to_owned()
        })
    );
    ended(&watch);
    talk.close();
}

#[test]
fn a_peer_that_keeps_printing_gets_no_longer_than_the_wait() {
    let _serial = serial();
    let folder = scratch();
    let (mut talk, watch) = holding(
        &folder,
        "read a\nyes '{\"method\":\"n\"}' &\nprinting=$!\nsleep 3\nkill $printing\n\
         echo '{\"id\":1,\"result\":1}'",
    );
    assert_eq!(
        talk.request("m", json!({})),
        Err(Fault::Silent {
            method: "m".to_owned()
        })
    );
    ended(&watch);
    talk.close();
}

#[test]
fn a_peer_that_ends_is_named_with_its_first_line_of_standard_error() {
    let _serial = serial();
    for (body, line) in [
        ("echo boom >&2\necho second >&2", "boom"),
        ("printf 'bo\\377om  \\n' >&2", "bo\u{fffd}om  "),
        ("printf 'boom\\r\\n' >&2", "boom"),
    ] {
        let folder = scratch();
        let mut talk = peer(&folder, &format!("read a\n{body}\nexit 0"));
        assert_eq!(
            talk.request("m", json!({})),
            Err(Fault::Ended {
                method: "m".to_owned(),
                line: Some(line.to_owned())
            }),
            "{body}"
        );
        talk.close();
    }
}

#[test]
fn a_peer_that_exited_while_its_output_stayed_open_is_said_to_have_ended() {
    let _serial = serial();
    let folder = scratch();
    let (mut talk, watch) = holding(&folder, "read a\necho boom >&2\nsleep 30 &\nexit 3");
    assert_eq!(
        talk.request("m", json!({})),
        Err(Fault::Ended {
            method: "m".to_owned(),
            line: Some("boom".to_owned())
        })
    );
    ended(&watch);
    talk.close();
}

#[test]
fn an_error_reply_is_said_in_its_message() {
    let _serial = serial();
    let folder = scratch();
    let mut talk = peer(
        &folder,
        "read a\necho '{\"id\":1,\"error\":{\"code\":1,\"message\":\"nope\"}}'",
    );
    assert_eq!(
        talk.request("m", json!({})),
        Err(Fault::Answered {
            method: "m".to_owned(),
            message: "nope".to_owned()
        })
    );
    talk.close();
}

#[test]
fn a_peer_that_closes_its_output_and_lives_does_not_hang() {
    let _serial = serial();
    let folder = scratch();
    let (mut talk, watch) = holding(&folder, "read a\nexec 1>&-\nwhile :; do sleep 30; done");
    let result = talk.request("m", json!({}));
    assert!(
        matches!(result, Err(Fault::Ended { line: None, .. })),
        "{result:?}"
    );
    ended(&watch);
    talk.close();
}

#[test]
fn a_peer_that_reads_nothing_is_said_not_to_take_the_request() {
    let _serial = serial();
    let folder = scratch();
    let (mut talk, watch) = holding(&folder, "sleep 30");
    assert_eq!(
        talk.request("m", json!({"pad": "x".repeat(1_000_000)})),
        Err(Fault::NotTaken {
            method: "m".to_owned()
        })
    );
    ended(&watch);
    drop(talk);
}

#[test]
fn closing_a_peer_waits_then_stops_it() {
    let _serial = serial();
    let folder = scratch();
    let (talk, watch) = holding(&folder, "while :; do sleep 30; done");
    talk.close();
    ended(&watch);
}

#[test]
fn closing_a_peer_stops_what_it_left_holding_its_output() {
    let _serial = serial();
    let folder = scratch();
    let held = fifo(&folder);
    let watch = watching(&held);
    let talk = peer_with(
        &folder,
        "(while :; do sleep 30; done) 3>\"$1\" &\nread a\nexit 0",
        &[held.to_str().expect("UTF-8")],
    );
    talk.close();
    ended(&watch);
}

#[test]
fn a_peer_that_speaks_before_it_reads_is_still_heard_and_answered() {
    let _serial = serial();
    let folder = scratch();
    let tool = script(
        &folder,
        "peer",
        "i=0\nwhile [ $i -lt 10000 ]; do echo '{\"method\":\"n\"}'; i=$((i+1)); done\n\
         read a\necho '{\"id\":1,\"result\":{\"ok\":true}}'",
    );
    let mut talk = Talk::start(&tool, &[], bounds(LIMIT)).expect("a peer starts");
    assert_eq!(
        talk.request("m", json!({"pad": "x".repeat(1_000_000)})),
        Ok(json!({"ok": true}))
    );
    talk.close();
}
