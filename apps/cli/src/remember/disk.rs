use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Files, Node};
use bilbo_store::entries::Stamp;

/// Reads the disk and never writes it.
pub struct Disk;

impl Files for Disk {
    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn node(&self, path: &Path) -> io::Result<Node> {
        let kind = fs::symlink_metadata(path)?.file_type();
        Ok(if kind.is_file() {
            Node::File
        } else if kind.is_dir() {
            Node::Folder
        } else {
            Node::Other
        })
    }

    fn list(&self, folder: &Path) -> io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(folder)? {
            names.push(entry?.file_name());
        }
        names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        Ok(names)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn stamp(&self, path: &Path) -> io::Result<Stamp> {
        let meta = fs::symlink_metadata(path)?;
        Ok(Stamp {
            size: meta.len(),
            modified: meta.modified()?,
        })
    }
}
