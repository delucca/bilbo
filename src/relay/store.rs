//! The relay's data folder: the transport tree at the paths a `file://` transport uses, `.relay.lock` held for the
//! relay's lifetime, and the durable create-only writer, which streams a body to `.tmp/`, flushes it, links it into
//! place only when the name is free, and flushes the folder before the relay answers.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The data folder, locked while this value lives.
pub struct Data {
    root: PathBuf,
    _lock: File,
}

/// How a create ended.
#[derive(Debug, PartialEq)]
pub enum Created {
    /// The object is new: 201.
    New,
    /// The object was there with the same bytes: 200, nothing changed.
    Same,
    /// The object was there with other bytes: 409 `exists`.
    Other,
    /// The disk or the quota is full: 507 `quota`.
    Full(String),
    /// Any other failure: 500 `internal`.
    Failed(String),
}

/// A body received under `.tmp/` and flushed, removed when dropped unless it was linked.
pub struct Staged {
    path: PathBuf,
    length: u64,
    sha256: String,
}

impl Data {
    /// Takes the data folder `root`: creates it with mode 0700 when it is missing, holds `.relay.lock` until the
    /// value is dropped (`another relay serves <root>` when another process holds it), and empties `.tmp/`.
    pub fn open(root: &Path) -> Result<Data, String> {
        let _ = root;
        unimplemented!("stream S")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Streams `body`, which must yield exactly `length` bytes, to a new file under `.tmp/`, and flushes it. A failure
    /// removes the file and is `Full` or `Failed`.
    pub fn stage(&self, length: u64, body: &mut dyn Read) -> Result<Staged, Created> {
        let _ = (length, body);
        unimplemented!("stream S")
    }

    /// The bytes at `path`, a path of the transport layout; `None` when nothing is there.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let _ = path;
        unimplemented!("stream S")
    }

    /// Removes `pair/<nameplate>/` and what it holds.
    pub fn remove_nameplate(&self, nameplate: &str) -> Result<(), String> {
        let _ = nameplate;
        unimplemented!("stream S")
    }
}

impl Staged {
    pub fn length(&self) -> u64 {
        self.length
    }

    /// The lowercase hex SHA-256 of the body.
    pub fn sha256_hex(&self) -> &str {
        &self.sha256
    }

    /// The body's bytes, for a manifest or a message, and for comparing with an object already there.
    pub fn bytes(&self) -> io::Result<Vec<u8>> {
        std::fs::read(&self.path)
    }

    /// Links the body to `path` of the layout in `data`: creates and flushes each missing folder, refuses a name that
    /// is taken (`Same` or `Other` by its bytes), and flushes the parent before it returns `New`.
    pub fn link(self, data: &Data, path: &str) -> Created {
        let _ = (data, path);
        unimplemented!("stream S")
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
