use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use bilbo_settings::edit::read_regular;

use crate::timer::{Files, LogPath};

fn cannot_read(path: &Path, error: &io::Error) -> String {
    format!("cannot read {}: {error}", path.display())
}

pub fn look(path: &Path) -> Result<LogPath, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Ok(LogPath::Link),
        Ok(meta) if meta.is_file() => Ok(LogPath::Regular),
        Ok(_) => Ok(LogPath::Other),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(LogPath::Nothing),
        Err(error) => Err(cannot_read(path, &error)),
    }
}

pub fn is_there(path: &Path) -> Result<bool, String> {
    Ok(look(path)? != LogPath::Nothing)
}

pub fn folder_is_there(folder: &Path) -> Result<bool, String> {
    match fs::metadata(folder) {
        Ok(meta) => Ok(meta.is_dir()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(cannot_read(folder, &error)),
    }
}

pub enum Unit {
    Wanted,
    Missing,
    Differs,
}

pub fn unit(path: &Path, text: &str) -> Result<Unit, String> {
    match look(path)? {
        LogPath::Nothing => return Ok(Unit::Missing),
        LogPath::Regular => {}
        LogPath::Link | LogPath::Other => {
            return Err(format!(
                "cannot read {}: not a regular file",
                path.display()
            ));
        }
    }
    let bytes = match read_regular(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Unit::Missing),
        Err(error) => return Err(cannot_read(path, &error)),
    };
    let writable = fs::metadata(path)
        .map_err(|error| cannot_read(path, &error))?
        .permissions()
        .mode()
        & 0o022
        != 0;
    Ok(if bytes == text.as_bytes() && !writable {
        Unit::Wanted
    } else {
        Unit::Differs
    })
}

pub fn files(units: &[(PathBuf, String)]) -> Result<Files, String> {
    let (mut wanted, mut missing) = (0, 0);
    for (path, text) in units {
        match unit(path, text)? {
            Unit::Wanted => wanted += 1,
            Unit::Missing => missing += 1,
            Unit::Differs => {}
        }
    }
    Ok(if wanted == units.len() {
        Files::Wanted
    } else if missing == units.len() {
        Files::Absent
    } else {
        Files::Differ
    })
}

pub fn delete(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot delete {}: {error}", path.display())),
    }
}

pub fn make_log_folder(log: &Path) -> Result<(), String> {
    let Some(folder) = log.parent() else {
        return Ok(());
    };
    let mut missing: Vec<&Path> = Vec::new();
    for ancestor in folder.ancestors() {
        match fs::metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == ErrorKind::NotFound => missing.push(ancestor),
            Err(error) => return Err(format!("cannot write {}: {error}", ancestor.display())),
        }
    }
    for ancestor in missing.into_iter().rev() {
        let cannot = |error: io::Error| format!("cannot write {}: {error}", ancestor.display());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(ancestor)
            .map_err(cannot)?;
        fs::set_permissions(ancestor, fs::Permissions::from_mode(0o700)).map_err(cannot)?;
    }
    Ok(())
}

pub fn write(path: &Path, text: &str) -> Result<(), String> {
    let cannot = |error: io::Error| format!("cannot write {}: {error}", path.display());
    let folder = path
        .parent()
        .ok_or_else(|| cannot(io::Error::other("it has no folder")))?;
    fs::create_dir_all(folder).map_err(cannot)?;
    let name = path
        .file_name()
        .ok_or_else(|| cannot(io::Error::other("it has no name")))?;
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = folder.join(temporary);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&temporary)
        .map_err(cannot)?;
    let renamed = (|| {
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    renamed.map_err(|error| {
        let _ = fs::remove_file(&temporary);
        cannot(error)
    })
}
