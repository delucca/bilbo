use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{Pid, Signal, getuid, kill_process_group};

#[derive(Debug, Clone, Copy)]
pub enum Environment<'a> {
    Inherit,
    Only(&'a [(&'a str, &'a str)]),
}

#[derive(Debug, PartialEq, Eq)]
pub enum End {
    Exited(i32),
    NoExitCode,
    TimedOut,
    OverLimit,
    NotStarted(String),
}

#[derive(Debug)]
pub struct Run {
    pub end: End,
    pub stdout: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub limit: usize,
    pub timeout: Duration,
    pub poll: Duration,
}

pub fn run(
    program: &Path,
    args: &[&str],
    environment: Environment<'_>,
    folder: &Path,
    stdin: &[u8],
    bounds: Bounds,
) -> io::Result<Run> {
    let limit = bounds.limit;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(folder)
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Environment::Only(variables) = environment {
        command.env_clear().envs(variables.iter().copied());
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Ok(Run {
                end: End::NotStarted(error.to_string()),
                stdout: Vec::new(),
            });
        }
    };
    let Some(group) = i32::try_from(child.id()).ok().and_then(Pid::from_raw) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("a process id that is not a pid"));
    };
    let (Some(mut input), Some(mut output)) = (child.stdin.take(), child.stdout.take()) else {
        stop(group, &mut child);
        let _ = child.wait();
        return Err(io::Error::other("a pipe of the child is missing"));
    };
    let bytes = stdin.to_vec();
    let writer = thread::spawn(move || input.write_all(&bytes));
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let over = Arc::new(AtomicBool::new(false));
    let reader = thread::spawn({
        let over = Arc::clone(&over);
        move || {
            let mut buffer = Vec::new();
            (&mut output).take(cap).read_to_end(&mut buffer)?;
            if buffer.len() > limit {
                over.store(true, Ordering::SeqCst);
            }
            Ok::<_, io::Error>(buffer)
        }
    });
    let watched = watch(
        &mut child,
        &over,
        Instant::now() + bounds.timeout,
        bounds.poll,
    );
    // A tool that exits can leave a process that holds the pipe, so the group is stopped on every end.
    stop(group, &mut child);
    let timed_out = matches!(watched, Ok(None)) && !over.load(Ordering::SeqCst);
    let waited = child.wait();
    let written = writer.join();
    let read = reader.join();
    let status = match watched {
        Ok(Some(status)) => status,
        Ok(None) => waited?,
        Err(error) => return Err(error),
    };
    match written {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.kind() == io::ErrorKind::BrokenPipe => {}
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(io::Error::other("the writer panicked")),
    }
    let stdout = read.map_err(|_| io::Error::other("the reader panicked"))??;
    let end = if stdout.len() > limit {
        End::OverLimit
    } else if timed_out {
        End::TimedOut
    } else {
        match status.code() {
            Some(code) => End::Exited(code),
            None => End::NoExitCode,
        }
    };
    Ok(Run { end, stdout })
}

fn watch(
    child: &mut Child,
    over: &AtomicBool,
    deadline: Instant,
    poll: Duration,
) -> io::Result<Option<ExitStatus>> {
    loop {
        let status = child.try_wait()?;
        let left = deadline.saturating_duration_since(Instant::now());
        if status.is_some() || over.load(Ordering::SeqCst) || left.is_zero() {
            return Ok(status);
        }
        thread::sleep(poll.min(left));
    }
}

fn stop(group: Pid, child: &mut Child) {
    if let Err(error) = kill_process_group(group, Signal::KILL)
        && error != Errno::SRCH
    {
        let _ = child.kill();
    }
}

pub fn folder(path: &Path) -> Result<(), String> {
    cleaned(path, getuid().as_raw())
}

pub fn own(path: &Path, meta: &fs::Metadata) -> Result<(), String> {
    owned(path, meta.uid(), getuid().as_raw())
}

fn cleaned(path: &Path, user: u32) -> Result<(), String> {
    let failed = |error: io::Error| format!("{}: {error}", path.display());
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .map_err(failed);
        }
        Err(error) => return Err(failed(error)),
    };
    if meta.file_type().is_symlink() {
        return Err(format!(
            "{} is a link: bilbo will not run a program there",
            path.display()
        ));
    }
    if !meta.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    owned(path, meta.uid(), user)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let kind = entry.file_type().map_err(failed)?;
        if kind.is_dir() && fs::read_dir(entry.path()).map_err(failed)?.next().is_some() {
            return Err(format!(
                "{} holds a folder that is not empty: {}",
                path.display(),
                entry.file_name().to_string_lossy()
            ));
        }
        entries.push((entry.path(), kind.is_dir()));
    }
    for (entry, is_dir) in entries {
        if is_dir {
            fs::remove_dir(entry).map_err(failed)?;
        } else {
            fs::remove_file(entry).map_err(failed)?;
        }
    }
    Ok(())
}

fn owned(path: &Path, owner: u32, user: u32) -> Result<(), String> {
    if owner == user {
        Ok(())
    } else {
        Err(format!("{} belongs to another user", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use std::fs;

    use rustix::process::getuid;

    use super::{cleaned, own, owned};

    #[test]
    fn a_folder_of_another_user_is_refused() {
        let path = Path::new("/some/folder");
        assert_eq!(
            owned(path, 501, 0).unwrap_err(),
            "/some/folder belongs to another user"
        );
        assert_eq!(owned(path, 501, 501), Ok(()));
    }

    #[test]
    fn a_folder_another_user_owns_is_refused_before_anything_is_removed() {
        let folder = std::env::temp_dir().join(format!("bilbo-child-owner-{}", std::process::id()));
        fs::create_dir_all(&folder).expect("a folder");
        fs::write(folder.join("held"), "x").expect("a file");
        let mine = getuid().as_raw();
        assert_eq!(
            cleaned(&folder, mine + 1).unwrap_err(),
            format!("{} belongs to another user", folder.display())
        );
        assert!(folder.join("held").exists());
        assert_eq!(cleaned(&folder, mine), Ok(()));
        assert!(!folder.join("held").exists());
        fs::remove_dir(&folder).expect("the folder");
    }

    #[test]
    fn a_folder_of_the_users_own_is_owned() {
        let folder = std::env::temp_dir().join(format!("bilbo-child-own-{}", std::process::id()));
        fs::create_dir_all(&folder).expect("a folder");
        let metadata = fs::symlink_metadata(&folder).expect("its metadata");
        assert_eq!(own(&folder, &metadata), Ok(()));
        fs::remove_dir(&folder).expect("the folder");
    }
}
