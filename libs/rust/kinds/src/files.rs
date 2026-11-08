use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    File,
    Folder,
    /// A symbolic link, socket or device.
    Other,
}

pub trait Files {
    /// Absolute, symbolic links resolved.
    fn canonical(&self, path: &Path) -> io::Result<PathBuf>;
    /// What `path` is, without following a symbolic link.
    fn node(&self, path: &Path) -> io::Result<Node>;
    /// The names in a folder, sorted bytewise.
    fn list(&self, folder: &Path) -> io::Result<Vec<OsString>>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
}

#[derive(Debug, Clone)]
enum Entry {
    File(Vec<u8>),
    Folder,
    Other,
    Unreadable(Node),
}

/// An in-memory tree of absolute paths, for the tests of the use case and of every kind crate.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    tree: BTreeMap<PathBuf, Entry>,
}

impl Memory {
    #[must_use]
    pub fn new() -> Memory {
        Memory::default()
    }

    /// A file, with every missing parent folder up to `/`.
    #[must_use]
    pub fn file(mut self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) -> Memory {
        let path = path.into();
        self.parents(&path);
        self.tree.insert(path, Entry::File(bytes.into()));
        self
    }

    #[must_use]
    pub fn folder(mut self, path: impl Into<PathBuf>) -> Memory {
        let path = path.into();
        self.parents(&path);
        self.tree.insert(path, Entry::Folder);
        self
    }

    /// A symbolic link, socket or device: `node()` is `Other`.
    #[must_use]
    pub fn other(mut self, path: impl Into<PathBuf>) -> Memory {
        let path = path.into();
        self.parents(&path);
        self.tree.insert(path, Entry::Other);
        self
    }

    /// `node()` says `node`; `list()` and `read()` fail with `PermissionDenied`.
    #[must_use]
    pub fn unreadable(mut self, path: impl Into<PathBuf>, node: Node) -> Memory {
        let path = path.into();
        self.parents(&path);
        self.tree.insert(path, Entry::Unreadable(node));
        self
    }

    fn parents(&mut self, path: &Path) {
        for parent in path.ancestors().skip(1) {
            self.tree
                .entry(parent.to_path_buf())
                .or_insert(Entry::Folder);
        }
    }

    fn entry(&self, path: &Path) -> io::Result<&Entry> {
        self.tree
            .get(path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

impl Files for Memory {
    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        if !path.is_absolute() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        self.entry(path).map(|_| path.to_path_buf())
    }

    fn node(&self, path: &Path) -> io::Result<Node> {
        Ok(match self.entry(path)? {
            Entry::File(_) => Node::File,
            Entry::Folder => Node::Folder,
            Entry::Other => Node::Other,
            Entry::Unreadable(node) => *node,
        })
    }

    fn list(&self, folder: &Path) -> io::Result<Vec<OsString>> {
        match self.entry(folder)? {
            Entry::Folder => {}
            Entry::Unreadable(_) => return Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            _ => return Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
        let mut names: Vec<OsString> = self
            .tree
            .keys()
            .filter(|path| path.parent() == Some(folder))
            .filter_map(|path| path.file_name().map(OsString::from))
            .collect();
        names.sort_by(|a, b| {
            std::os::unix::ffi::OsStrExt::as_bytes(a.as_os_str())
                .cmp(std::os::unix::ffi::OsStrExt::as_bytes(b.as_os_str()))
        });
        Ok(names)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.entry(path)? {
            Entry::File(bytes) => Ok(bytes.clone()),
            Entry::Unreadable(_) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};

    use super::{Files, Memory, Node};

    #[test]
    fn memory_answers_like_a_disk_without_following_links() {
        let files = Memory::new()
            .file("/t/b.md", "b")
            .file("/t/a.md", "a")
            .file("/t/sub/c.md", "c")
            .other("/t/link.md")
            .unreadable("/t/locked", Node::Folder)
            .unreadable("/t/secret.md", Node::File);

        assert_eq!(
            files.canonical(Path::new("/t/a.md")).ok(),
            Some(PathBuf::from("/t/a.md"))
        );
        assert_eq!(
            files.canonical(Path::new("/t/none")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );
        assert_eq!(
            files.canonical(Path::new("t/a.md")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );

        assert_eq!(files.node(Path::new("/t")).ok(), Some(Node::Folder));
        assert_eq!(files.node(Path::new("/t/a.md")).ok(), Some(Node::File));
        assert_eq!(files.node(Path::new("/t/link.md")).ok(), Some(Node::Other));
        assert_eq!(files.node(Path::new("/t/locked")).ok(), Some(Node::Folder));
        assert_eq!(files.node(Path::new("/t/secret.md")).ok(), Some(Node::File));
        assert_eq!(
            files.node(Path::new("/nope")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );

        let names: Vec<OsString> = ["a.md", "b.md", "link.md", "locked", "secret.md", "sub"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(files.list(Path::new("/t")).ok(), Some(names));
        assert_eq!(
            files.list(Path::new("/t/sub")).ok(),
            Some(vec![OsString::from("c.md")])
        );
        assert_eq!(
            files.list(Path::new("/t/locked")).map_err(|e| e.kind()),
            Err(ErrorKind::PermissionDenied)
        );
        assert_eq!(
            files.list(Path::new("/t/a.md")).map_err(|e| e.kind()),
            Err(ErrorKind::InvalidInput)
        );
        assert_eq!(
            files.list(Path::new("/nope")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );

        assert_eq!(files.read(Path::new("/t/a.md")).ok(), Some(b"a".to_vec()));
        assert_eq!(
            files.read(Path::new("/t")).map_err(|e| e.kind()),
            Err(ErrorKind::InvalidInput)
        );
        assert_eq!(
            files.read(Path::new("/t/link.md")).map_err(|e| e.kind()),
            Err(ErrorKind::InvalidInput)
        );
        assert_eq!(
            files.read(Path::new("/t/secret.md")).map_err(|e| e.kind()),
            Err(ErrorKind::PermissionDenied)
        );
        assert_eq!(
            files.read(Path::new("/nope")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );
    }
}
