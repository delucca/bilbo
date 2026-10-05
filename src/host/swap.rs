//! Atomic file exchange, no-replace rename and the access check, the calls std does not expose.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("swap needs renamex_np (macOS) or renameat2 (Linux)");

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// The message for a filesystem that refuses both calls; restore compares against it.
pub const UNSUPPORTED: &str = "it cannot swap files atomically";

/// Swaps two existing paths in one step.
pub fn exchange(a: &Path, b: &Path) -> Result<(), String> {
    call(a, b, Flag::Exchange)
}

/// Renames `a` to `b` only when `b` does not exist.
pub fn rename_new(a: &Path, b: &Path) -> Result<(), String> {
    call(a, b, Flag::NoReplace)
}

enum Flag {
    Exchange,
    NoReplace,
}

#[cfg(target_os = "macos")]
const SWAP: libc::c_uint = libc::RENAME_SWAP;
#[cfg(target_os = "macos")]
const EXCL: libc::c_uint = libc::RENAME_EXCL;
#[cfg(target_os = "linux")]
const SWAP: libc::c_uint = libc::RENAME_EXCHANGE;
#[cfg(target_os = "linux")]
const EXCL: libc::c_uint = libc::RENAME_NOREPLACE;

fn call(a: &Path, b: &Path, flag: Flag) -> Result<(), String> {
    let from = c_path(a)?;
    let to = c_path(b)?;
    let flags = match flag {
        Flag::Exchange => SWAP,
        Flag::NoReplace => EXCL,
    };
    // SAFETY: from and to are NUL-terminated and outlive the call.
    #[cfg(target_os = "macos")]
    let rc = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), flags) };
    // SAFETY: from and to are NUL-terminated and outlive the call.
    #[cfg(target_os = "linux")]
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            flags,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::ENOTSUP) => Err(UNSUPPORTED.to_string()),
        _ => Err(match flag {
            Flag::Exchange => format!("cannot swap {} with {}: {err}", a.display(), b.display()),
            Flag::NoReplace => format!("cannot move {} to {}: {err}", a.display(), b.display()),
        }),
    }
}

/// Whether this process may write to `path`, by `access(2)`: ownership, ACLs and read-only mounts count, and
/// nothing is created.
pub fn writable(path: &Path) -> bool {
    let Ok(path) = c_path(path) else {
        return false;
    };
    // SAFETY: path is NUL-terminated and outlives the call.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("{} holds a NUL byte", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-swap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    #[test]
    fn exchange_swaps_two_files() {
        let d = dir("exchange_swaps_two_f");
        let (a, b) = (d.0.join("a"), d.0.join("b"));
        fs::write(&a, "one").unwrap();
        fs::write(&b, "two").unwrap();
        let (ino_a, ino_b) = (
            fs::metadata(&a).unwrap().ino(),
            fs::metadata(&b).unwrap().ino(),
        );
        exchange(&a, &b).unwrap();
        assert_eq!(fs::metadata(&a).unwrap().ino(), ino_b);
        assert_eq!(fs::metadata(&b).unwrap().ino(), ino_a);
        assert_eq!(fs::read_to_string(&a).unwrap(), "two");
        assert_eq!(fs::read_to_string(&b).unwrap(), "one");
    }

    #[test]
    fn exchange_with_a_missing_side_fails_and_changes_nothing() {
        let d = dir("exchange_with_a_miss");
        let (a, b) = (d.0.join("a"), d.0.join("b"));
        fs::write(&a, "one").unwrap();
        let message = exchange(&a, &b).unwrap_err();
        assert!(message.ends_with(&format!("(os error {})", libc::ENOENT)));
        assert_eq!(fs::read_to_string(&a).unwrap(), "one");
        assert!(!b.exists());
    }

    #[test]
    fn rename_new_moves_onto_a_free_name() {
        let d = dir("rename_new_moves_ont");
        let (a, b) = (d.0.join("a"), d.0.join("b"));
        fs::write(&a, "one").unwrap();
        rename_new(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read_to_string(&b).unwrap(), "one");
    }

    #[test]
    fn rename_new_refuses_a_taken_name() {
        let d = dir("rename_new_refuses_a");
        let (a, b) = (d.0.join("a"), d.0.join("b"));
        fs::write(&a, "one").unwrap();
        fs::write(&b, "two").unwrap();
        let message = rename_new(&a, &b).unwrap_err();
        assert!(message.ends_with(&format!("(os error {})", libc::EEXIST)));
        assert_eq!(fs::read_to_string(&a).unwrap(), "one");
        assert_eq!(fs::read_to_string(&b).unwrap(), "two");
    }

    #[test]
    fn writable_asks_the_system_and_creates_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("writable_asks_the_s");
        assert!(writable(&d.0));
        assert!(!writable(&d.0.join("missing")));
        assert!(!d.0.join("missing").exists());
        let locked = d.0.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        // SAFETY: geteuid takes no arguments and cannot fail.
        let root = unsafe { libc::geteuid() } == 0;
        assert_eq!(writable(&locked), root);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    }
}
