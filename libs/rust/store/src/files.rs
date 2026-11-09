use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use crate::entries::StoreError;

pub fn io_err(action: &'static str, path: &Path, error: io::Error) -> StoreError {
    StoreError::Io {
        action,
        path: path.to_path_buf(),
        error,
    }
}

pub fn create_dir(path: &Path) -> Result<(), StoreError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| io_err("create", path, error))
}

/// Whether the group may write the folder harmlessly: it cannot, or it is the user's own
/// effective group (most Linux users have a private primary group and a umask of 002).
fn group_is_safe(meta: &fs::Metadata) -> bool {
    meta.mode() & 0o020 == 0 || meta.gid() == rustix::process::getegid().as_raw()
}

/// Refuses a folder that another user owns, that others can write, or that a group other than
/// the user's effective group can write: they could plant links in it. A folder that is gone is
/// an error, so a check after creation proves it exists.
pub fn check_private(path: &Path) -> Result<(), StoreError> {
    let meta = fs::metadata(path).map_err(|error| io_err("read", path, error))?;
    if meta.mode() & 0o002 == 0
        && group_is_safe(&meta)
        && meta.uid() == rustix::process::geteuid().as_raw()
    {
        Ok(())
    } else {
        Err(StoreError::Unsafe(path.to_path_buf()))
    }
}

/// `check_private` for a folder that may not exist.
pub fn check_private_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => check_private(path),
    }
}

/// `path` with its deepest existing ancestor made canonical and the missing part appended.
pub fn resolve(path: &Path) -> Result<std::path::PathBuf, StoreError> {
    let mut missing = Vec::new();
    let mut at = path;
    loop {
        match fs::canonicalize(at) {
            Ok(mut resolved) => {
                resolved.extend(missing.iter().rev());
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match (at.parent(), at.file_name()) {
                    (Some(parent), Some(name)) => {
                        missing.push(name);
                        at = parent;
                    }
                    _ => return Err(io_err("read", at, error)),
                }
            }
            Err(error) => return Err(io_err("read", at, error)),
        }
    }
}

const MAX_LINKS: usize = 40;

enum Part {
    Root,
    Parent,
    Name(OsString),
}

fn parts(path: &Path) -> impl Iterator<Item = Part> {
    path.components().rev().filter_map(|part| match part {
        Component::Prefix(_) | Component::RootDir => Some(Part::Root),
        Component::CurDir => None,
        Component::ParentDir => Some(Part::Parent),
        Component::Normal(name) => Some(Part::Name(name.to_os_string())),
    })
}

/// Where `path` leads, following every link in it, a dangling one included: a link to a missing
/// target counts as that target. `resolve` keeps a dangling link as it is, which hides that a
/// writer would create its target.
pub fn follow(path: &Path) -> Result<PathBuf, StoreError> {
    let mut todo: Vec<Part> = parts(path).collect();
    let mut at = if path.is_absolute() {
        PathBuf::new()
    } else {
        fs::canonicalize(".").map_err(|error| io_err("read", Path::new("."), error))?
    };
    let mut links = 0;
    while let Some(part) = todo.pop() {
        match part {
            Part::Root => at = PathBuf::from("/"),
            Part::Parent => {
                at.pop();
            }
            Part::Name(name) => {
                let next = at.join(name);
                match fs::symlink_metadata(&next) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        links += 1;
                        if links > MAX_LINKS {
                            return Err(io_err(
                                "read",
                                path,
                                io::Error::other("too many levels of symbolic links"),
                            ));
                        }
                        let target =
                            fs::read_link(&next).map_err(|error| io_err("read", &next, error))?;
                        todo.extend(parts(&target));
                    }
                    Ok(_) => at = next,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => at = next,
                    Err(error) => return Err(io_err("read", &next, error)),
                }
            }
        }
    }
    Ok(at)
}

/// Refuses a path with an ancestor that another user (root excepted) owns, or that others or a
/// group other than the effective group can write unless it is sticky: anyone who could rename
/// such an ancestor could swap the folders under it. With every ancestor guarded, a folder that passed its own check stays put.
pub fn check_parents(path: &Path) -> Result<(), StoreError> {
    let uid = rustix::process::geteuid().as_raw();
    for ancestor in resolve(path)?.ancestors().skip(1) {
        let meta = match fs::metadata(ancestor) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_err("read", ancestor, error)),
        };
        let mode = meta.mode();
        let owned = meta.uid() == uid || meta.uid() == 0;
        let open = mode & 0o1000 == 0 && (mode & 0o002 != 0 || !group_is_safe(&meta));
        if !owned || open {
            return Err(StoreError::Unsafe(ancestor.to_path_buf()));
        }
    }
    Ok(())
}

pub fn sync_dir(path: &Path) -> Result<(), StoreError> {
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| io_err("flush", path, error))
}

pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_err("read", path, error)),
    }
}

pub fn read_file(path: &Path) -> Result<Vec<u8>, StoreError> {
    fs::read(path).map_err(|error| io_err("read", path, error))
}

pub fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_err("remove", path, error)),
    }
}

pub fn private_file() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.mode(0o600);
    options
}

#[derive(Debug, Clone, Copy)]
pub enum Create {
    /// The file must not exist.
    New,
    /// The file is made or emptied.
    Replace,
}

/// Writes `bytes` to `path` and syncs the file.
pub fn write_synced(path: &Path, bytes: &[u8], how: Create) -> Result<(), StoreError> {
    let mut options = private_file();
    options.write(true);
    match how {
        Create::New => options.create_new(true),
        Create::Replace => options.create(true).truncate(true),
    };
    let mut file = options
        .open(path)
        .map_err(|error| io_err("create", path, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_err("write", path, error))
}
