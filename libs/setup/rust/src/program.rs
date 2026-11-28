use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{Access, access};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group};

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub wait: Duration,
    pub poll: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum End {
    Exited(i32),
    NoExitCode,
    StillRunning,
    OutputOpen,
    Unreadable(String),
    NotStarted(String),
}

#[derive(Debug)]
pub struct Ran {
    pub end: End,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn runnable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file()) && access(path, Access::EXEC_OK).is_ok()
}

pub fn find(value: impl AsRef<OsStr>, path: impl AsRef<OsStr>) -> Option<PathBuf> {
    let value = value.as_ref();
    if value.as_bytes().contains(&b'/') {
        let given = PathBuf::from(value);
        return runnable(&given).then_some(given);
    }
    path.as_ref()
        .as_bytes()
        .split(|byte| *byte == b':')
        .map(|folder| Path::new(OsStr::from_bytes(folder)))
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(value))
        .find(|candidate| runnable(candidate))
}

trait Target {
    fn kill_group(&mut self) -> Result<(), Errno>;
    fn kill_itself(&mut self) -> io::Result<()>;
    fn reap(&mut self) -> bool;
}

struct Process {
    child: Child,
    group: Option<Pid>,
}

impl Process {
    fn new(child: Child) -> Process {
        let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
        Process { child, group }
    }
}

impl Target for Process {
    fn kill_group(&mut self) -> Result<(), Errno> {
        match self.group {
            Some(group) => kill_process_group(group, Signal::KILL),
            None => Err(Errno::SRCH),
        }
    }

    fn kill_itself(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    fn reap(&mut self) -> bool {
        self.child.wait().is_ok()
    }
}

fn stop(target: &mut impl Target) {
    let _ = target.kill_group();
    if target.kill_itself().is_ok() && target.reap() {
        let _ = target.kill_group();
    }
}

fn started(program: &Path, args: &[&str]) -> io::Result<Child> {
    Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn drain(mut stream: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn reading(stream: impl Read + Send + 'static) -> Receiver<io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(drain(stream));
    });
    receiver
}

fn looked(
    status: Option<&ExitStatus>,
    stdout: &mut Option<io::Result<Vec<u8>>>,
    stderr: &mut Option<io::Result<Vec<u8>>>,
) -> Option<Ran> {
    let broken = [&*stdout, &*stderr]
        .into_iter()
        .find_map(|read| match read {
            Some(Err(error)) => Some(error.to_string()),
            _ => None,
        });
    if let Some(error) = broken {
        return Some(Ran {
            end: End::Unreadable(error),
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
    }
    let status = status?;
    if stdout.is_none() || stderr.is_none() {
        return None;
    }
    let (Some(Ok(stdout)), Some(Ok(stderr))) = (stdout.take(), stderr.take()) else {
        return None;
    };
    Some(Ran {
        end: status.code().map_or(End::NoExitCode, End::Exited),
        stdout,
        stderr,
    })
}

pub fn run(program: &Path, args: &[&str], bounds: Bounds) -> Ran {
    let mut child = match started(program, args) {
        Ok(child) => child,
        Err(error) => {
            return Ran {
                end: End::NotStarted(error.to_string()),
                stdout: Vec::new(),
                stderr: Vec::new(),
            };
        }
    };
    drop(child.stdin.take());
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        stop(&mut Process::new(child));
        return Ran {
            end: End::NotStarted("a pipe of the program is missing".to_owned()),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
    };
    let mut process = Process::new(child);
    let (out, err) = (reading(out), reading(err));
    let deadline = Instant::now() + bounds.wait;
    let (mut status, mut stdout, mut stderr) = (None, None, None);
    loop {
        if status.is_none() {
            status = process.child.try_wait().ok().flatten();
        }
        if stdout.is_none() {
            stdout = out.try_recv().ok();
        }
        if stderr.is_none() {
            stderr = err.try_recv().ok();
        }
        if let Some(ran) = looked(status.as_ref(), &mut stdout, &mut stderr) {
            if matches!(ran.end, End::Unreadable(_)) {
                stop(&mut process);
            }
            return ran;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        thread::sleep(bounds.poll.min(left));
    }
    let end = if status.is_none() {
        End::StillRunning
    } else {
        End::OutputOpen
    };
    stop(&mut process);
    Ran {
        end,
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

pub type Judge<'a> = dyn FnMut(&[u8]) -> bool + 'a;

pub enum Written {
    Running,
    Ended(Duration),
    Failed,
}

pub enum Next {
    Line(Vec<u8>),
    Nothing,
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Said {
    Reply(Vec<u8>),
    Sent,
    NotTaken,
    Failed,
    Closed,
    RanOut,
}

pub trait Wire {
    fn begin(&mut self);
    fn write(&mut self, within: Duration) -> Written;
    fn next(&mut self, within: Duration) -> Next;
    fn now(&mut self) -> Duration;
}

pub fn exchange(
    wire: &mut dyn Wire,
    end: Duration,
    poll: Duration,
    mut is_reply: Option<&mut Judge>,
) -> Said {
    if wire.now() >= end {
        return Said::NotTaken;
    }
    wire.begin();
    let (mut ended, mut closed, mut reply) = (false, false, None);
    loop {
        if !ended {
            let now = wire.now();
            let within = if closed {
                poll.min(end.saturating_sub(now))
            } else {
                Duration::ZERO
            };
            match wire.write(within) {
                Written::Failed => return Said::Failed,
                Written::Ended(at) if at > end => return Said::NotTaken,
                Written::Ended(_) => ended = true,
                Written::Running => {}
            }
        }
        if ended {
            match (&is_reply, reply.take()) {
                (None, _) => return Said::Sent,
                (Some(_), Some(line)) => return Said::Reply(line),
                (Some(_), None) => {}
            }
        }
        let now = wire.now();
        if closed && is_reply.is_some() && reply.is_none() && now < end {
            return Said::Closed;
        }
        if now >= end {
            let written = if ended {
                Written::Ended(Duration::ZERO)
            } else {
                wire.write(Duration::ZERO)
            };
            return match written {
                Written::Failed => Said::Failed,
                Written::Ended(at) if at <= end => match (&is_reply, reply) {
                    (Some(_), Some(line)) => Said::Reply(line),
                    (None, _) => Said::Sent,
                    (Some(_), None) => Said::RanOut,
                },
                Written::Ended(_) | Written::Running => Said::NotTaken,
            };
        }
        if closed {
            continue;
        }
        match wire.next(poll.min(end - now)) {
            Next::Closed => closed = true,
            Next::Nothing => {}
            Next::Line(line) => {
                let judged = reply.is_none() && is_reply.as_mut().is_some_and(|judge| judge(&line));
                if judged && wire.now() < end {
                    reply = Some(line);
                }
            }
        }
    }
}

struct Live<'a> {
    start: Instant,
    input: Option<ChildStdin>,
    bytes: Option<Vec<u8>>,
    result: Option<Receiver<(ChildStdin, io::Result<()>, Instant)>>,
    lines: &'a Receiver<Vec<u8>>,
}

impl Wire for Live<'_> {
    fn begin(&mut self) {
        let (Some(mut input), Some(bytes)) = (self.input.take(), self.bytes.take()) else {
            return;
        };
        let (done, result) = mpsc::channel();
        self.result = Some(result);
        thread::spawn(move || {
            let written = input.write_all(&bytes).and_then(|()| input.flush());
            let _ = done.send((input, written, Instant::now()));
        });
    }

    fn write(&mut self, within: Duration) -> Written {
        let Some(result) = &self.result else {
            return Written::Failed;
        };
        let answer = if within.is_zero() {
            result.try_recv().map_err(|error| match error {
                TryRecvError::Empty => RecvTimeoutError::Timeout,
                TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            })
        } else {
            result.recv_timeout(within)
        };
        match answer {
            Ok((input, Ok(()), at)) => {
                self.input = Some(input);
                Written::Ended(at.saturating_duration_since(self.start))
            }
            Ok((_, Err(_), _)) | Err(RecvTimeoutError::Disconnected) => Written::Failed,
            Err(RecvTimeoutError::Timeout) => Written::Running,
        }
    }

    fn next(&mut self, within: Duration) -> Next {
        match self.lines.recv_timeout(within) {
            Ok(line) => Next::Line(line),
            Err(RecvTimeoutError::Timeout) => Next::Nothing,
            Err(RecvTimeoutError::Disconnected) => Next::Closed,
        }
    }

    fn now(&mut self) -> Duration {
        self.start.elapsed()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lost {
    NotTaken,
    Silent,
    Ended(Option<String>),
}

pub struct Peer {
    process: Process,
    input: Option<ChildStdin>,
    lines: Receiver<Vec<u8>>,
    error: Receiver<Option<String>>,
    first: Option<Option<String>>,
    bounds: Bounds,
    stopped: bool,
}

fn first_line(mut line: Vec<u8>) -> Option<String> {
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    let text = String::from_utf8_lossy(&line).into_owned();
    (!text.is_empty()).then_some(text)
}

pub fn first_stderr_line(stderr: &[u8]) -> Option<String> {
    let end = stderr
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(stderr.len(), |at| at + 1);
    first_line(stderr[..end].to_vec())
}

impl Peer {
    pub fn start(program: &Path, args: &[&str], bounds: Bounds) -> io::Result<Peer> {
        let mut child = started(program, args)?;
        let (Some(input), Some(out), Some(err)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            stop(&mut Process::new(child));
            return Err(io::Error::other("a pipe of the program is missing"));
        };
        let (sender, lines) = mpsc::sync_channel(0);
        thread::spawn(move || {
            let mut out = BufReader::new(out);
            loop {
                let mut line = Vec::new();
                match out.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if line.last() == Some(&b'\n') {
                            line.pop();
                        }
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let (first, error) = mpsc::channel();
        thread::spawn(move || {
            let mut err = BufReader::new(err);
            let mut line = Vec::new();
            let _ = err.read_until(b'\n', &mut line);
            let _ = first.send(first_line(line));
            let _ = io::copy(&mut err, &mut io::sink());
        });
        Ok(Peer {
            process: Process::new(child),
            input: Some(input),
            lines,
            error,
            first: None,
            bounds,
            stopped: false,
        })
    }

    fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;
            self.input = None;
            stop(&mut self.process);
        }
    }

    fn first_line(&mut self, deadline: Instant) -> Option<String> {
        if let Some(first) = &self.first {
            return first.clone();
        }
        let wait = deadline.saturating_duration_since(Instant::now()) + self.bounds.poll;
        match self.error.recv_timeout(wait) {
            Ok(line) => {
                self.first = Some(line.clone());
                line
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.first = Some(None);
                None
            }
            Err(RecvTimeoutError::Timeout) => None,
        }
    }

    fn ended(&mut self, deadline: Instant) -> Lost {
        self.stop();
        Lost::Ended(self.first_line(deadline))
    }

    fn lapsed(&mut self, deadline: Instant) -> Lost {
        let exited = self.process.child.try_wait().ok().flatten().is_some();
        if exited {
            self.ended(deadline)
        } else {
            self.stop();
            Lost::Silent
        }
    }

    pub fn exchange(
        &mut self,
        bytes: &[u8],
        deadline: Instant,
        is_reply: Option<&mut Judge>,
    ) -> Result<Option<Vec<u8>>, Lost> {
        let Some(input) = self.input.take() else {
            return Err(self.ended(deadline));
        };
        let start = Instant::now();
        let end = deadline.saturating_duration_since(start);
        let mut wire = Live {
            start,
            input: Some(input),
            bytes: Some(bytes.to_vec()),
            result: None,
            lines: &self.lines,
        };
        let said = match is_reply {
            Some(judge) => exchange(&mut wire, end, self.bounds.poll, Some(judge)),
            None => exchange(&mut wire, end, self.bounds.poll, None),
        };
        self.input = wire.input.take();
        match said {
            Said::Reply(line) => Ok(Some(line)),
            Said::Sent => Ok(None),
            Said::NotTaken => {
                self.stop();
                Err(Lost::NotTaken)
            }
            Said::Failed | Said::Closed => Err(self.ended(deadline)),
            Said::RanOut => Err(self.lapsed(deadline)),
        }
    }

    pub fn close(mut self) {
        if self.stopped {
            return;
        }
        drop(self.input.take());
        let bounds = self.bounds;
        let deadline = Instant::now() + bounds.wait;
        let (mut exited, mut out_ended, mut err_ended) = (false, false, false);
        loop {
            exited = exited || self.process.child.try_wait().ok().flatten().is_some();
            while !out_ended {
                match self.lines.try_recv() {
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => out_ended = true,
                }
            }
            while !err_ended {
                match self.error.try_recv() {
                    Ok(line) => self.first = Some(line),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => err_ended = true,
                }
            }
            if exited && out_ended && err_ended {
                self.stopped = true;
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.stop();
                return;
            }
            thread::sleep(bounds.poll.min(left));
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use rustix::io::Errno;

    use std::collections::VecDeque;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::time::Duration;

    use super::{End, Next, Said, Target, Wire, Written, drain, exchange, looked, stop};

    struct Stand {
        group: Result<(), Errno>,
        itself: bool,
        reaped: bool,
        log: Vec<&'static str>,
    }

    impl Target for Stand {
        fn kill_group(&mut self) -> Result<(), Errno> {
            self.log.push("group");
            self.group
        }

        fn kill_itself(&mut self) -> io::Result<()> {
            self.log.push("itself");
            if self.itself {
                Ok(())
            } else {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }

        fn reap(&mut self) -> bool {
            self.log.push("reap");
            self.reaped
        }
    }

    #[test]
    fn the_program_is_killed_and_reaped_whatever_its_group_answers() {
        for group in [Ok(()), Err(Errno::SRCH), Err(Errno::PERM)] {
            let mut stand = Stand {
                group,
                itself: true,
                reaped: true,
                log: Vec::new(),
            };
            stop(&mut stand);
            assert_eq!(stand.log, ["group", "itself", "reap", "group"], "{group:?}");
        }
        let mut unreaped = Stand {
            group: Ok(()),
            itself: true,
            reaped: false,
            log: Vec::new(),
        };
        stop(&mut unreaped);
        assert_eq!(unreaped.log, ["group", "itself", "reap"]);
        let mut refused = Stand {
            group: Ok(()),
            itself: false,
            reaped: true,
            log: Vec::new(),
        };
        stop(&mut refused);
        assert_eq!(refused.log, ["group", "itself"]);
    }

    struct Breaks(u8);

    impl Read for Breaks {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0 == 0 {
                return Err(io::Error::other("the pipe broke"));
            }
            let given = buffer.len().min(usize::from(self.0));
            buffer[..given].fill(b'x');
            self.0 -= u8::try_from(given).unwrap_or(0);
            Ok(given)
        }
    }

    #[test]
    fn a_stream_that_fails_part_way_is_an_error_and_not_its_first_bytes() {
        let error = drain(Breaks(5)).expect_err("a failure");
        assert!(error.to_string().contains("the pipe broke"), "{error}");
        assert_eq!(drain(&b"xxxxx"[..]).expect("five bytes"), b"xxxxx");
    }

    struct Fake {
        count: Duration,
        writes: VecDeque<(Written, Duration)>,
        lines: VecDeque<(Next, Duration)>,
        begun: bool,
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    impl Fake {
        fn new(writes: Vec<(Written, u64)>, lines: Vec<(Next, u64)>) -> Fake {
            for (write, at) in &writes {
                if let Written::Ended(ended) = write {
                    assert!(*ended <= secs(*at), "an end handed before its moment");
                }
            }
            Fake {
                count: Duration::ZERO,
                writes: writes.into_iter().map(|(w, at)| (w, secs(at))).collect(),
                lines: lines.into_iter().map(|(l, at)| (l, secs(at))).collect(),
                begun: false,
            }
        }
    }

    impl Wire for Fake {
        fn begin(&mut self) {
            self.begun = true;
        }

        fn write(&mut self, within: Duration) -> Written {
            match self.writes.pop_front() {
                Some((write, at)) => {
                    self.count = self.count.max(at);
                    write
                }
                None => {
                    self.count += within;
                    Written::Running
                }
            }
        }

        fn next(&mut self, within: Duration) -> Next {
            match self.lines.pop_front() {
                Some((next, at)) => {
                    self.count = self.count.max(at);
                    next
                }
                None => {
                    self.count += within;
                    Next::Nothing
                }
            }
        }

        fn now(&mut self) -> Duration {
            self.count
        }
    }

    const POLL: Duration = Duration::from_secs(1);

    fn notice(fake: &mut Fake, end: u64) -> Said {
        exchange(fake, secs(end), POLL, None)
    }

    fn request(fake: &mut Fake, end: u64) -> Said {
        let mut judge = |line: &[u8]| line == b"reply";
        exchange(fake, secs(end), POLL, Some(&mut judge))
    }

    fn line(text: &str) -> Next {
        Next::Line(text.as_bytes().to_vec())
    }

    fn reply() -> Said {
        Said::Reply(b"reply".to_vec())
    }

    #[test]
    fn a_write_counts_only_when_it_ended_within_its_wait() {
        let mut none = Fake::new(vec![], vec![]);
        assert_eq!(notice(&mut none, 0), Said::NotTaken);
        assert!(!none.begun, "no write was begun");

        let mut late = Fake::new(
            vec![(Written::Running, 0), (Written::Ended(secs(12)), 12)],
            vec![],
        );
        assert_eq!(notice(&mut late, 10), Said::NotTaken);

        let mut timely = Fake::new(
            vec![(Written::Running, 0), (Written::Ended(secs(3)), 3)],
            vec![],
        );
        assert_eq!(notice(&mut timely, 10), Said::Sent);
    }

    #[test]
    fn a_reply_found_after_the_wait_ran_out_is_not_taken() {
        for (when, expected) in [(11, Said::RanOut), (5, reply())] {
            let mut fake = Fake::new(
                vec![(Written::Ended(secs(1)), 1)],
                vec![(line("notice"), 1), (line("reply"), when)],
            );
            assert_eq!(request(&mut fake, 10), expected, "{when}");
        }
    }

    #[test]
    fn a_reply_or_a_notice_waits_for_its_write_when_the_peer_closes() {
        let mut asked = Fake::new(
            vec![
                (Written::Running, 0),
                (Written::Running, 1),
                (Written::Ended(secs(1)), 3),
            ],
            vec![(line("reply"), 1), (Next::Closed, 2)],
        );
        assert_eq!(request(&mut asked, 10), reply());

        let mut sent = Fake::new(
            vec![(Written::Running, 0), (Written::Ended(secs(2)), 2)],
            vec![(Next::Closed, 1)],
        );
        assert_eq!(notice(&mut sent, 10), Said::Sent);

        let mut never = Fake::new(vec![(Written::Running, 0)], vec![(Next::Closed, 1)]);
        assert_eq!(notice(&mut never, 10), Said::NotTaken);

        let mut unanswered = Fake::new(
            vec![(Written::Running, 0), (Written::Running, 1)],
            vec![(Next::Closed, 2)],
        );
        assert_eq!(request(&mut unanswered, 10), Said::Closed);

        let mut closed_at_the_end =
            Fake::new(vec![(Written::Running, 0)], vec![(Next::Closed, 10)]);
        assert_eq!(request(&mut closed_at_the_end, 10), Said::NotTaken);
    }

    #[test]
    fn a_late_reply_does_not_hide_an_unfinished_write() {
        let mut never = Fake::new(vec![(Written::Running, 0)], vec![(line("reply"), 11)]);
        assert_eq!(request(&mut never, 10), Said::NotTaken);

        let mut seen_late = Fake::new(
            vec![(Written::Running, 0), (Written::Ended(secs(9)), 10)],
            vec![(line("reply"), 5)],
        );
        assert_eq!(request(&mut seen_late, 10), reply());
    }

    fn exited(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    #[test]
    fn a_stream_that_failed_ends_the_run_at_once() {
        let broke = || Some(Err(std::io::Error::other("the pipe broke")));
        let ok = || Some(Ok(b"x".to_vec()));
        let words = |ran: Option<super::Ran>| ran.map(|ran| ran.end);
        let broken = Some(End::Unreadable("the pipe broke".to_owned()));
        assert_eq!(words(looked(None, &mut broke(), &mut None)), broken);
        assert_eq!(words(looked(None, &mut None, &mut broke())), broken);
        let status = exited(3);
        let ran = looked(Some(&status), &mut ok(), &mut ok()).expect("an end");
        assert_eq!((ran.end, ran.stdout), (End::Exited(3), b"x".to_vec()));
        assert!(
            looked(None, &mut ok(), &mut ok()).is_none(),
            "the program still runs"
        );
        assert!(
            looked(Some(&status), &mut ok(), &mut None).is_none(),
            "a stream not yet read"
        );
        assert!(looked(Some(&status), &mut None, &mut None).is_none());
    }
}
