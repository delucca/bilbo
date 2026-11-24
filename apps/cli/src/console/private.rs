use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rustix::fs::{AtFlags, FileType, Mode, OFlags, mkdirat, openat, statat, unlinkat};
use rustix::process::geteuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Read,
    ReadAsOpened,
    Edited,
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

fn refused_folder(path: &Path) -> io::Error {
    io::Error::other(Refused(format!(
        "{} is not a private folder of this user",
        path.display()
    )))
}

pub fn is_refusal(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<Refused>().is_some())
}

fn ordinary(meta: &fs::Metadata) -> bool {
    meta.is_file() && meta.mode() & 0o077 == 0 && meta.uid() == geteuid().as_raw()
}

pub fn private(meta: &fs::Metadata) -> bool {
    ordinary(meta) && meta.nlink() == 1
}

pub fn checked(file: &fs::File, path: &Path) -> io::Result<()> {
    if private(&file.metadata()?) {
        Ok(())
    } else {
        Err(refused(path))
    }
}

pub fn checked_for(file: &fs::File, path: &Path, how: How) -> io::Result<()> {
    let meta = match how {
        How::ReadAsOpened | How::Edited => file.metadata()?,
        _ => return checked(file, path),
    };
    let held = match how {
        How::Edited => meta.is_file() && meta.uid() == geteuid().as_raw() && meta.nlink() == 1,
        _ => ordinary(&meta) && meta.nlink() <= 1,
    };
    if held { Ok(()) } else { Err(refused(path)) }
}

pub fn folder(parent: &fs::File, name: &str, path: &Path) -> io::Result<fs::File> {
    mkdirat(parent, name, Mode::from_raw_mode(0o700))?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let seen = openat(parent, name, flags, Mode::empty())
        .map(fs::File::from)
        .map_err(io::Error::from)
        .and_then(|made| {
            let meta = made.metadata()?;
            if meta.is_dir() && meta.uid() == geteuid().as_raw() && meta.mode() & 0o777 == 0o700 {
                Ok(made)
            } else {
                Err(refused_folder(path))
            }
        });
    if seen.is_err() {
        let _ = unlinkat(parent, name, AtFlags::REMOVEDIR);
    }
    seen
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
            How::Read | How::ReadAsOpened | How::Edited => OFlags::RDONLY,
            How::Append => OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE,
            How::Write => OFlags::WRONLY | OFlags::CREATE,
            How::New => OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
        };
    let file = fs::File::from(openat(folder, name, flags, Mode::from_raw_mode(0o600))?);
    checked_for(&file, path, how)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};

    use std::io::Read;

    use super::{How, checked, checked_for, folder as private_folder, is_refusal, open};

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

    fn sentence(path: &Path) -> String {
        format!(
            "{} is not a private file of this user in a folder of this user",
            path.display()
        )
    }

    fn mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("a mode");
    }

    #[test]
    fn a_private_file_that_lost_its_name_is_taken_as_it_was_opened_and_by_no_other_way() {
        let (base, dir) = folder("as-opened");
        let said = |name: &str| base.join(name);
        for replaced in [false, true] {
            let name = if replaced { "renamed" } else { "removed" };
            open(&dir, name, &said(name), How::New).expect("a file");
            fs::write(said(name), "v0").expect("a file");
            let held = fs::File::open(said(name)).expect("held");
            if replaced {
                open(&dir, "next", &said("next"), How::New).expect("a file");
                fs::write(said("next"), "v1").expect("a file");
                fs::rename(said("next"), said(name)).expect("a rename");
                assert_eq!(fs::read_to_string(said(name)).expect("read"), "v1");
            } else {
                fs::remove_file(said(name)).expect("removed");
            }
            checked_for(&held, &said(name), How::ReadAsOpened).expect(name);
            let mut text = String::new();
            (&held).read_to_string(&mut text).expect("read");
            assert_eq!(text, "v0");
            for how in [How::Read, How::Append, How::Write, How::New] {
                let error = checked_for(&held, &said(name), how).expect_err(name);
                assert!(is_refusal(&error), "{how:?}");
                assert_eq!(error.to_string(), sentence(&said(name)));
            }
            let error = checked(&held, &said(name)).expect_err(name);
            assert!(is_refusal(&error));
            assert_eq!(error.to_string(), sentence(&said(name)));
        }
        let _ = fs::remove_dir_all(Path::new(&base));
    }

    #[test]
    fn a_file_that_is_not_otherwise_private_is_refused_as_it_was_opened_too() {
        let (base, dir) = folder("as-opened-refused");
        let said = |name: &str| base.join(name);
        let refused_as_opened = |held: &fs::File, name: &str| {
            let error = checked_for(held, &said(name), How::ReadAsOpened).expect_err(name);
            assert!(is_refusal(&error), "{name}");
            assert_eq!(error.to_string(), sentence(&said(name)), "{name}");
        };
        fs::write(said("gone"), "x").expect("a file");
        mode(&said("gone"), 0o644);
        let held = fs::File::open(said("gone")).expect("held");
        fs::remove_file(said("gone")).expect("removed");
        refused_as_opened(&held, "gone");
        fs::write(said("over"), "x").expect("a file");
        mode(&said("over"), 0o644);
        let held = fs::File::open(said("over")).expect("held");
        open(&dir, "next", &said("next"), How::New).expect("a file");
        fs::rename(said("next"), said("over")).expect("a rename");
        refused_as_opened(&held, "over");
        fs::write(said("kept"), "x").expect("a file");
        mode(&said("kept"), 0o644);
        refused_as_opened(&fs::File::open(said("kept")).expect("held"), "kept");
        open(&dir, "mine", &said("mine"), How::New).expect("a file");
        fs::hard_link(said("mine"), said("twin")).expect("a second name");
        refused_as_opened(&fs::File::open(said("twin")).expect("held"), "twin");
        symlink(said("mine"), said("link")).expect("a link");
        for name in ["link", "twin", "kept"] {
            let error = open(&dir, name, &said(name), How::ReadAsOpened).expect_err(name);
            assert_eq!(error.to_string(), sentence(&said(name)), "{name}");
        }
        fs::remove_file(said("twin")).expect("removed");
        fs::write(said("mine"), "v0").expect("a file");
        let mut text = String::new();
        open(&dir, "mine", &said("mine"), How::ReadAsOpened)
            .expect("a private file")
            .read_to_string(&mut text)
            .expect("read");
        assert_eq!(text, "v0");
        let error = open(&dir, "none", &said("none"), How::ReadAsOpened).expect_err("none");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        let _ = fs::remove_dir_all(Path::new(&base));
    }

    #[test]
    fn a_file_an_editor_wrote_anew_is_read_whatever_its_mode() {
        let (base, dir) = folder("edited");
        let said = |name: &str| base.join(name);
        fs::write(said("saved"), "text").expect("a file");
        mode(&said("saved"), 0o644);
        let mut text = String::new();
        open(&dir, "saved", &said("saved"), How::Edited)
            .expect("a file of mode 0644")
            .read_to_string(&mut text)
            .expect("read");
        assert_eq!(text, "text");
        let error = open(&dir, "saved", &said("saved"), How::Read).expect_err("an open mode");
        assert_eq!(error.to_string(), sentence(&said("saved")));

        fs::hard_link(said("saved"), said("twin")).expect("a second name");
        symlink(said("saved"), said("link")).expect("a link");
        fs::create_dir(said("dir")).expect("a folder");
        for name in ["saved", "twin", "link", "dir"] {
            let error = open(&dir, name, &said(name), How::Edited).expect_err(name);
            assert!(is_refusal(&error), "{name}");
            assert_eq!(error.to_string(), sentence(&said(name)), "{name}");
        }
        fs::remove_file(said("twin")).expect("one name again");
        let error = open(&dir, "none", &said("none"), How::Edited).expect_err("none");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);

        let held = fs::File::open(said("saved")).expect("held");
        checked_for(&held, &said("saved"), How::Edited).expect("one name");
        fs::remove_file(said("saved")).expect("removed");
        let error = checked_for(&held, &said("saved"), How::Edited).expect_err("no name left");
        assert!(is_refusal(&error));
        assert_eq!(error.to_string(), sentence(&said("saved")));
        let _ = fs::remove_dir_all(Path::new(&base));
    }

    #[test]
    fn a_private_folder_is_made_new_or_not_at_all() {
        use std::os::unix::fs::MetadataExt;

        let (base, dir) = folder("private-folder");
        let said = |name: &str| base.join(name);
        let made = private_folder(&dir, "free", &said("free")).expect("a new folder");
        let meta = made.metadata().expect("meta");
        assert!(meta.is_dir());
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        assert_eq!(meta.uid(), rustix::process::geteuid().as_raw());
        open(&made, "note.md", &said("free").join("note.md"), How::New).expect("a file in it");
        assert!(said("free").join("note.md").is_file());

        fs::create_dir(said("taken")).expect("a folder");
        fs::write(said("taken").join("kept"), "x").expect("a file");
        fs::write(said("file"), "y").expect("a file");
        fs::create_dir(said("target")).expect("a folder");
        symlink(said("target"), said("link")).expect("a link");
        for name in ["taken", "file", "link"] {
            let error = private_folder(&dir, name, &said(name)).expect_err(name);
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{name}");
        }
        assert_eq!(
            fs::read_to_string(said("taken").join("kept")).expect("as it was"),
            "x"
        );
        assert_eq!(fs::read_to_string(said("file")).expect("as it was"), "y");
        assert!(said("link").is_symlink());
        assert_eq!(fs::read_dir(said("target")).expect("as it was").count(), 0);
        let _ = fs::remove_dir_all(Path::new(&base));
    }
}
