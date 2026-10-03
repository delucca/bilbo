use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a finished program left: its exit code (`None` when a signal ended it) and its output, lossily decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Runs another program; tests script it, `System` runs it for real.
pub trait Runner {
    /// `Err` only when the program could not be started; the message names it.
    fn run(&self, program: &Path, args: &[String]) -> Result<Output, String>;
}

/// Runs programs with stdin closed and bilbo's own environment.
pub struct System;

impl Runner for System {
    fn run(&self, program: &Path, args: &[String]) -> Result<Output, String> {
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
        Ok(Output {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// The first executable regular file called `name` in the absolute folders of `path` (a PATH value).
pub fn find(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|file| is_executable(file))
}

/// A regular file (after links) with an execute bit set.
pub fn is_executable(file: &Path) -> bool {
    std::fs::metadata(file)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The first line of `text` that is not blank, trimmed; empty when there is none.
pub fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

/// What `agents` asks of a JSON-RPC peer; `Rpc` is the real one, tests script it.
pub trait Calls {
    /// The `result` of `method`, or a message naming the error.
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

/// A child spoken to in JSON lines: requests with ids 1, 2, ... on stdin, replies matched by id on
/// stdout, anything else on stdout skipped. Dropping it closes stdin and kills the child.
pub struct Rpc {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: std::sync::mpsc::Receiver<String>,
    stderr: Option<std::thread::JoinHandle<String>>,
    name: String,
    next: u64,
    limit: std::time::Duration,
}

impl Rpc {
    /// Starts `program args`; every call waits at most `limit` for its reply.
    pub fn start(program: &Path, args: &[&str], limit: std::time::Duration) -> Result<Rpc, String> {
        use std::io::{BufRead, Read};
        let name = format!("{} {}", program.display(), args.join(" "));
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).split(b'\n') {
                let Ok(line) = line else { break };
                if tx
                    .send(String::from_utf8_lossy(&line).into_owned())
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut err = child.stderr.take().unwrap();
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            text
        });
        Ok(Rpc {
            stdin: child.stdin.take(),
            child,
            lines,
            stderr: Some(stderr),
            name,
            next: 0,
            limit,
        })
    }

    pub fn notify(&mut self, method: &str) -> Result<(), String> {
        self.send(&serde_json::json!({ "method": method }))
    }

    fn send(&mut self, message: &serde_json::Value) -> Result<(), String> {
        use std::io::Write;
        let stdin = self.stdin.as_mut().ok_or("stdin closed")?;
        writeln!(stdin, "{message}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("{} stopped reading: {e}", self.name))
    }

    /// Closes stdin and kills the child if it still runs, then reaps it.
    fn stop(&mut self) {
        self.stdin = None;
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }

    /// Closes stdin and waits up to the limit for the child to exit.
    pub fn finish(mut self) {
        self.stdin = None;
        let start = std::time::Instant::now();
        while start.elapsed() < self.limit {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Calls for Rpc {
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.next += 1;
        let id = self.next;
        self.send(&serde_json::json!({ "id": id, "method": method, "params": params }))?;
        let deadline = std::time::Instant::now() + self.limit;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    self.stop();
                    return Err(format!(
                        "{} did not answer {method} within {} s",
                        self.name,
                        self.limit.as_secs()
                    ));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.stop();
                    let stderr = self
                        .stderr
                        .take()
                        .and_then(|reader| reader.join().ok())
                        .unwrap_or_default();
                    let why = first_line(&stderr);
                    return Err(format!(
                        "{} ended before answering {method}{}",
                        self.name,
                        if why.is_empty() {
                            String::new()
                        } else {
                            format!(": {why}")
                        }
                    ));
                }
            };
            let Ok(reply) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if reply.get("id").and_then(serde_json::Value::as_u64) != Some(id)
                || reply.get("method").is_some()
            {
                continue;
            }
            if let Some(error) = reply.get("error") {
                let message = error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("an error");
                return Err(format!("{} answered {method}: {message}", self.name));
            }
            return Ok(reply
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null));
        }
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-command-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn file(path: &Path, mode: u32) {
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn find_takes_the_first_executable() {
        let s = scratch("find");
        let (a, b) = (s.0.join("a"), s.0.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        file(&a.join("tool"), 0o644);
        file(&b.join("tool"), 0o755);
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find("tool", Some(&path)), Some(b.join("tool")));
    }

    #[test]
    fn find_skips_folders_and_relative_entries() {
        let s = scratch("skip");
        std::fs::create_dir_all(s.0.join("tool")).unwrap();
        let path = OsString::from(format!("relative:{}", s.0.display()));
        assert_eq!(find("tool", Some(&path)), None);
        assert_eq!(find("tool", None), None);
    }

    #[test]
    fn system_runs_a_program() {
        let out = System
            .run(
                Path::new("/bin/sh"),
                &["-c".into(), "echo out; echo err >&2; exit 3".into()],
            )
            .unwrap();
        assert_eq!(
            out,
            Output {
                code: Some(3),
                stdout: "out\n".into(),
                stderr: "err\n".into()
            }
        );
    }

    #[test]
    fn a_missing_program_is_named() {
        let err = System.run(Path::new("/nope/tool"), &[]).unwrap_err();
        assert!(err.starts_with("cannot run /nope/tool: "), "{err}");
    }

    fn sh(script: &str, limit: std::time::Duration) -> Rpc {
        Rpc::start(Path::new("/bin/sh"), &["-c", script], limit).unwrap()
    }

    const SECOND: std::time::Duration = std::time::Duration::from_secs(1);

    #[test]
    fn rpc_matches_replies_by_id_and_skips_notifications() {
        let mut rpc = sh(
            r#"read a; echo '{"method":"note","params":{}}'; echo 'not json'; echo '{"id":9,"result":"other"}'; echo '{"id":1,"method":"ask","params":{}}'; echo '{"id":1,"result":{"x":1}}'; read b; echo '{"id":2,"result":"two"}'"#,
            SECOND,
        );
        assert_eq!(
            rpc.call("one", serde_json::json!({})),
            Ok(serde_json::json!({"x": 1}))
        );
        assert_eq!(
            rpc.call("two", serde_json::json!({})),
            Ok(serde_json::json!("two"))
        );
        rpc.finish();
    }

    #[test]
    fn rpc_sends_notifications_without_an_id_and_requests_with_counting_ids() {
        let s = scratch("rpc-sent");
        let log = s.0.join("log");
        let script = format!(
            r#"read a; read b; read c; printf '%s\n%s\n%s\n' "$a" "$b" "$c" >{}; echo '{{"id":1,"result":null}}'; echo '{{"id":2,"result":null}}'; read d"#,
            log.display()
        );
        let mut rpc = sh(&script, SECOND);
        rpc.notify("ready").unwrap();
        // Replies for ids 1 and 2 are only read once the matching request is made.
        rpc.send(&serde_json::json!({"method": "extra"})).unwrap();
        assert!(rpc.call("one", serde_json::json!([1])).is_ok());
        assert!(rpc.call("two", serde_json::json!({})).is_ok());
        rpc.finish();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "{\"method\":\"ready\"}\n{\"method\":\"extra\"}\n{\"id\":1,\"method\":\"one\",\"params\":[1]}\n"
        );
    }

    #[test]
    fn rpc_reports_an_error_reply() {
        let mut rpc = sh(
            r#"read a; echo '{"id":1,"error":{"code":-32603,"message":"it broke"}}'"#,
            SECOND,
        );
        let err = rpc.call("m", serde_json::json!({})).unwrap_err();
        assert!(err.ends_with(" answered m: it broke"), "{err}");
    }

    #[test]
    fn rpc_times_out() {
        let mut rpc = sh("read a; read b", SECOND);
        let err = rpc.call("m", serde_json::json!({})).unwrap_err();
        assert!(err.ends_with(" did not answer m within 1 s"), "{err}");
    }

    #[test]
    fn rpc_child_that_closes_stdout_but_lives_does_not_hang() {
        let (tx, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut rpc = sh("read a; exec >&-; read b", SECOND);
            let _ = tx.send(rpc.call("m", serde_json::json!({})));
        });
        let reply = done
            .recv_timeout(std::time::Duration::from_secs(8))
            .expect("call hung");
        assert!(reply.is_err(), "{reply:?}");
    }

    #[test]
    fn rpc_reads_a_reply_after_a_line_that_is_not_utf8() {
        let mut rpc = sh(
            r#"read a; printf '\377\n'; echo '{"id":1,"result":1}'"#,
            SECOND,
        );
        assert_eq!(
            rpc.call("m", serde_json::json!({})),
            Ok(serde_json::json!(1))
        );
    }

    #[test]
    fn rpc_finish_after_a_timeout_does_not_wait_another_window() {
        let limit = std::time::Duration::from_millis(500);
        let mut rpc = sh("read a; read b; sleep 30", limit);
        assert!(rpc.call("m", serde_json::json!({})).is_err());
        let start = std::time::Instant::now();
        rpc.finish();
        assert!(start.elapsed() < limit / 2, "{:?}", start.elapsed());
    }

    #[test]
    fn rpc_names_a_child_that_ends() {
        let mut rpc = sh("read a; echo boom >&2; echo more >&2; exit 3", SECOND);
        let err = rpc.call("m", serde_json::json!({})).unwrap_err();
        assert!(err.ends_with(" ended before answering m: boom"), "{err}");
        let mut quiet = sh("exit 0", SECOND);
        let err = quiet.call("m", serde_json::json!({})).unwrap_err();
        assert!(err.ends_with(" ended before answering m"), "{err}");
    }

    #[test]
    fn rpc_start_names_a_missing_program() {
        let err = Rpc::start(Path::new("/nope/tool"), &["x"], SECOND)
            .err()
            .unwrap();
        assert!(err.starts_with("cannot run /nope/tool: "), "{err}");
    }

    #[test]
    fn first_line_skips_blank_lines() {
        assert_eq!(first_line("\n  \n Error: boom \nmore"), "Error: boom");
        assert_eq!(first_line(""), "");
    }
}
