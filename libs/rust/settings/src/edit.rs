//! The config file on disk: the loader's read, and every byte bilbo writes to it.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rustix::fs::{FileType, Mode, OFlags};

/// A file's bytes, read only from a regular file: a named pipe or a device would block the read.
pub fn read_regular(path: &Path) -> io::Result<Vec<u8>> {
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let stat = rustix::fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::from(fd).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Whether the file exists. A link (a Nix-managed config), a read-only file or another account's
/// file is refused: replacing it would cut it from what manages it.
pub fn check(config: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(config) {
        Ok(meta) if meta.file_type().is_symlink() => Err("it is a symbolic link".to_owned()),
        Ok(meta) if !meta.is_file() => Err("it is not a regular file".to_owned()),
        Ok(_) => fs::OpenOptions::new()
            .write(true)
            .open(config)
            .map(|_| true)
            .map_err(|_| "it is read-only".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

/// The bytes once `check` passes; none when there is no file.
pub fn read(config: &Path) -> Result<Vec<u8>, String> {
    if check(config)? {
        fs::read(config).map_err(|error| error.to_string())
    } else {
        Ok(Vec::new())
    }
}

/// `<config>.lock` beside the file, created private with its folder, never removed. The caller
/// holds the lock from its read through `replace`.
pub fn lock_file(config: &Path) -> io::Result<fs::File> {
    let folder = config
        .parent()
        .ok_or_else(|| io::Error::other("it has no folder"))?;
    fs::create_dir_all(folder)?;
    let mut name = config
        .file_name()
        .ok_or_else(|| io::Error::other("it has no name"))?
        .to_owned();
    name.push(".lock");
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(folder.join(name))
}

/// `tables` alone, or after `before` and a blank line.
pub fn appended(before: &[u8], tables: &str) -> Vec<u8> {
    let mut text = before.to_vec();
    if !text.is_empty() {
        if !text.ends_with(b"\n") {
            text.push(b'\n');
        }
        text.push(b'\n');
    }
    text.extend_from_slice(tables.as_bytes());
    text
}

/// Whether the key declared at `line` holds an inline value: only the `[[name]]` form starts it
/// on a `[[` header.
pub fn inline(before: &[u8], line: usize) -> bool {
    before
        .split(|&byte| byte == b'\n')
        .nth(line.saturating_sub(1))
        .is_some_and(|text| !text.trim_ascii_start().starts_with(b"[["))
}

/// Temp file then rename, keeping the old permissions; a failed folder sync is returned, not
/// refused.
pub fn replace(config: &Path, text: &[u8]) -> io::Result<Option<String>> {
    let folder = config
        .parent()
        .ok_or_else(|| io::Error::other("it has no folder"))?;
    let name = config
        .file_name()
        .ok_or_else(|| io::Error::other("it has no name"))?;
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = folder.join(temporary);
    let renamed = (|| {
        let mut file = fs::File::create_new(&temporary)?;
        if let Ok(meta) = fs::metadata(config) {
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(text)?;
        file.sync_all()?;
        fs::rename(&temporary, config)
    })();
    if renamed.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    finish(renamed, || fs::File::open(folder)?.sync_all())
}

fn finish(
    renamed: io::Result<()>,
    sync: impl FnOnce() -> io::Result<()>,
) -> io::Result<Option<String>> {
    renamed?;
    Ok(sync().err().map(|error| error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{appended, finish, inline};

    #[test]
    fn the_tables_go_after_the_file_as_it_was_with_a_blank_line_between() {
        let tables = "[[follow]]\npath = \"/x\"\n";
        assert_eq!(appended(b"", tables), tables.as_bytes());
        assert_eq!(
            appended(b"[store]\ncompression = 3\n", tables),
            b"[store]\ncompression = 3\n\n[[follow]]\npath = \"/x\"\n"
        );
        assert_eq!(
            appended(b"# no newline at the end", tables),
            b"# no newline at the end\n\n[[follow]]\npath = \"/x\"\n"
        );
        let odd = b"[store]  \r\n#  spaced   comment\r\n\n\n";
        assert!(appended(odd, tables).starts_with(odd));
    }

    #[test]
    fn a_failed_folder_sync_after_the_rename_is_returned_not_refused() {
        assert_eq!(
            finish(Ok(()), || Err(io::Error::other("no sync"))).expect("renamed"),
            Some("no sync".to_owned())
        );
        assert!(
            finish(Err(io::Error::other("no rename")), || panic!(
                "never synced"
            ))
            .is_err()
        );
    }

    #[test]
    fn only_a_double_bracket_header_at_the_line_is_not_inline() {
        assert!(!inline(b"# mine\n[[follow]]\npath = \"/x\"\n", 2));
        assert!(!inline(b"  [[follow]]\npath = \"/x\"\n", 1));
        assert!(inline(b"follow = [{ path = \"/x\" }]\n", 1));
        assert!(inline(b"# mine\nfollow = [\n  { path = \"/x\" },\n]\n", 2));
        assert!(!inline(b"follow = []\n", 3), "a line past the end");
    }
}
