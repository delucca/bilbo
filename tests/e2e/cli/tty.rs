use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::io::read;
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

use super::binary;

const ENV: [(&str, &str); 2] = [("TERM", "xterm-256color"), ("LANG", "en_US.UTF-8")];

const DEADLINE: Duration = Duration::from_secs(10);

enum Tty {
    Stdout,
    Stderr,
}

#[derive(Debug)]
struct Seen {
    code: i32,
    piped: String,
    terminal: String,
}

fn capture(command: Command, tty: &Tty) -> Seen {
    capture_within(command, tty, DEADLINE)
}

fn capture_within(mut command: Command, tty: &Tty, deadline: Duration) -> Seen {
    command.stdin(Stdio::null());
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("a pty master");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let name = ptsname(&master, Vec::new()).expect("a pty name");
    let slave: File = OpenOptions::new()
        .read(true)
        .write(true)
        .open(OsStr::from_bytes(name.to_bytes()))
        .expect("the pty slave");
    let on_terminal = || Stdio::from(slave.try_clone().expect("a slave handle"));
    match tty {
        Tty::Stdout => command.stdout(on_terminal()).stderr(Stdio::piped()),
        Tty::Stderr => command.stdout(Stdio::piped()).stderr(on_terminal()),
    };
    let mut child = command.spawn().expect("the command starts");
    let mut pipe: Box<dyn Read + Send> = match tty {
        Tty::Stdout => Box::new(child.stderr.take().expect("a piped stderr")),
        Tty::Stderr => Box::new(child.stdout.take().expect("a piped stdout")),
    };
    let piped = thread::spawn(move || {
        let mut text = Vec::new();
        pipe.read_to_end(&mut text).expect("reads the pipe");
        text
    });
    let flags = fcntl_getfl(&master).expect("flags");
    fcntl_setfl(&master, flags | OFlags::NONBLOCK).expect("nonblocking");
    let started = Instant::now();
    let mut terminal = Vec::new();
    let mut buffer = [0u8; 4096];
    let status = loop {
        // Asked before the drain: whatever the command wrote before it exited is then drained.
        let exited = child.try_wait().expect("try_wait");
        let wait = Timespec {
            tv_sec: 0,
            tv_nsec: 20_000_000,
        };
        poll(&mut [PollFd::new(&master, PollFlags::IN)], Some(&wait)).expect("poll");
        let mut got = 0;
        // The slave stays open here, so a drained master would block, not end.
        while let Ok(count) = read(&master, &mut buffer) {
            terminal.extend_from_slice(&buffer[..count]);
            got += count;
        }
        match exited {
            Some(status) if got == 0 => break status,
            Some(_) => {}
            None if started.elapsed() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "the command is still running after {deadline:?}; the terminal had {} bytes: {:?}",
                    terminal.len(),
                    String::from_utf8_lossy(&terminal)
                );
            }
            None => {}
        }
    };
    drop(slave);
    Seen {
        code: status.code().expect("an exit code"),
        piped: String::from_utf8(piped.join().expect("the pipe reader")).expect("UTF-8"),
        terminal: String::from_utf8(terminal)
            .expect("UTF-8")
            .replace("\r\n", "\n"),
    }
}

fn run(args: &[&str], tty: &Tty) -> Seen {
    let mut command = Command::new(binary());
    command.env_clear().envs(ENV).args(args);
    capture(command, tty)
}

#[test]
fn an_error_on_a_terminal_stderr_is_marked_while_a_piped_stdout_stays_plain() {
    let seen = run(&["frob"], &Tty::Stderr);
    assert_eq!(seen.code, 2);
    assert_eq!(seen.piped, "");
    assert_eq!(
        seen.terminal,
        "\x1b[31m■\x1b[39m  unknown verb 'frob'\n   \x1b[2musage: bilbo <verb> [<args>]...\x1b[22m\n   \x1b[2msee\x1b[22m \x1b[36mbilbo --help\x1b[39m\n"
    );
}

#[test]
fn an_error_on_a_piped_stderr_is_plain_while_a_terminal_stdout_is_untouched() {
    let seen = run(&["frob"], &Tty::Stdout);
    assert_eq!(seen.code, 2);
    assert_eq!(seen.terminal, "");
    assert_eq!(
        seen.piped,
        "bilbo: unknown verb 'frob'\nbilbo: usage: bilbo <verb> [<args>]...\nbilbo: see 'bilbo --help'\n"
    );
}

#[test]
fn help_on_a_terminal_stdout_has_bold_headings() {
    let seen = run(&["--help"], &Tty::Stdout);
    assert_eq!(seen.code, 0);
    assert_eq!(seen.piped, "");
    assert!(
        seen.terminal.contains("\x1b[1mUsage:\x1b[22m"),
        "{}",
        seen.terminal
    );
}

#[test]
fn help_on_a_piped_stdout_has_no_escape_byte_while_stderr_is_a_terminal() {
    let seen = run(&["--help"], &Tty::Stderr);
    assert_eq!(seen.code, 0);
    assert_eq!(seen.terminal, "");
    assert!(!seen.piped.contains('\x1b'), "{}", seen.piped);
    assert!(seen.piped.starts_with("bilbo keeps durable memory"));
}

#[test]
fn a_terminal_stream_larger_than_the_pty_buffer_is_read_while_the_command_runs() {
    let line = "0123456789012345678901234567890123456789012345678901234567890123";
    for (tty, redirect) in [(Tty::Stdout, ""), (Tty::Stderr, ">&2")] {
        let mut command = Command::new("/bin/sh");
        command.env_clear().args([
            "-c",
            &format!("i=0; while [ $i -lt 128 ]; do echo {line} {redirect}; i=$((i+1)); done"),
        ]);
        let seen = capture(command, &tty);
        assert_eq!(seen.code, 0);
        assert_eq!(seen.piped, "");
        assert_eq!(seen.terminal.len(), 128 * (line.len() + 1));
    }
}

#[test]
fn a_command_that_outlives_the_deadline_is_killed_and_the_test_fails() {
    let mut command = Command::new("/bin/sh");
    command
        .env_clear()
        .args(["-c", "echo pid=$$; exec /bin/sleep 37"]);
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        capture_within(command, &Tty::Stdout, Duration::from_millis(500))
    }))
    .expect_err("a command that never ends fails the test");
    let message = failure
        .downcast_ref::<String>()
        .expect("a formatted panic message");
    let pid = message
        .split("pid=")
        .nth(1)
        .and_then(|rest| rest.split("\\r").next())
        .expect("the command printed its pid");
    let alive = Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .expect("kill runs");
    assert!(!alive.success(), "process {pid} is still running");
}

#[test]
fn a_write_just_before_the_command_exits_is_not_lost() {
    let mut command = Command::new("/bin/sh");
    command
        .env_clear()
        .args(["-c", "/bin/sleep 0.2; echo late"]);
    let seen = capture(command, &Tty::Stdout);
    assert_eq!(seen.terminal, "late\n");
}

#[test]
fn a_command_gets_no_stdin_from_the_test() {
    let mut command = Command::new("/bin/cat");
    command.env_clear();
    let seen = capture(command, &Tty::Stdout);
    assert_eq!(seen.code, 0);
    assert_eq!(seen.terminal, "");
}
