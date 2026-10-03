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

    #[test]
    fn first_line_skips_blank_lines() {
        assert_eq!(first_line("\n  \n Error: boom \nmore"), "Error: boom");
        assert_eq!(first_line(""), "");
    }
}
