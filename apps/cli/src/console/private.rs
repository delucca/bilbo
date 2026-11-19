use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat};
use rustix::process::geteuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Read,
    Append,
    Write,
    New,
}

#[derive(Debug)]
struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

pub fn refused(path: &Path) -> io::Error {
    io::Error::other(Refused(format!(
        "{} is not a private file of this user in a folder of this user",
        path.display()
    )))
}

pub fn is_refusal(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<Refused>().is_some())
}

pub fn private(meta: &fs::Metadata) -> bool {
    meta.is_file()
        && meta.mode() & 0o077 == 0
        && meta.nlink() == 1
        && meta.uid() == geteuid().as_raw()
}

pub fn checked(file: &fs::File, path: &Path) -> io::Result<()> {
    if private(&file.metadata()?) {
        Ok(())
    } else {
        Err(refused(path))
    }
}

pub fn open(folder: &fs::File, name: &str, path: &Path, how: How) -> io::Result<fs::File> {
    match statat(folder, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(found) if FileType::from_raw_mode(found.st_mode) != FileType::RegularFile => {
            return Err(refused(path));
        }
        Ok(_) | Err(rustix::io::Errno::NOENT) => {}
        Err(error) => return Err(error.into()),
    }
    let flags = OFlags::NOFOLLOW
        | OFlags::NONBLOCK
        | OFlags::CLOEXEC
        | match how {
            How::Read => OFlags::RDONLY,
            How::Append => OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE,
            How::Write => OFlags::WRONLY | OFlags::CREATE,
            How::New => OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
        };
    let file = fs::File::from(openat(folder, name, flags, Mode::from_raw_mode(0o600))?);
    checked(&file, path)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};

    use super::{How, open};

    fn folder(test: &str) -> (PathBuf, fs::File) {
        let base = std::env::temp_dir().join(format!("private-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("a folder");
        let open = fs::File::open(&base).expect("an open folder");
        (base, open)
    }

    #[test]
    fn a_file_is_made_private_and_a_link_a_second_name_or_an_open_mode_is_refused() {
        let (base, dir) = folder("kinds");
        let said = |name: &str| base.join(name);
        let made = open(&dir, "mine", &said("mine"), How::Append).expect("a new file");
        assert_eq!(
            made.metadata().expect("meta").permissions().mode() & 0o777,
            0o600
        );
        symlink(said("mine"), said("link")).expect("a link");
        fs::hard_link(said("mine"), said("twin")).expect("a second name");
        fs::write(said("open"), "x").expect("a file");
        fs::set_permissions(said("open"), fs::Permissions::from_mode(0o644)).expect("a mode");
        let refused = |name: &str| {
            open(&dir, name, &said(name), How::Read)
                .expect_err(name)
                .to_string()
        };
        for name in ["link", "twin", "mine", "open"] {
            assert_eq!(
                refused(name),
                format!(
                    "{} is not a private file of this user in a folder of this user",
                    said(name).display()
                )
            );
        }
        let _ = fs::remove_dir_all(Path::new(&base));
    }
}
