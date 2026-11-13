use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bilbo_store::entries::Stamp;

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
    /// Without following a symbolic link.
    fn stamp(&self, path: &Path) -> io::Result<Stamp>;
    /// Separate so a fake can fix it.
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Debug, Clone)]
enum Entry {
    File(Vec<u8>, SystemTime),
    Folder,
    Other,
    Unreadable(Node),
    Denied,
}

/// An in-memory tree of absolute paths, for the tests of the use case and of every kind crate.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    tree: BTreeMap<PathBuf, Entry>,
    clock: Option<SystemTime>,
}

impl Memory {
    #[must_use]
    pub fn new() -> Memory {
        Memory::default()
    }

    #[must_use]
    pub fn file(mut self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) -> Memory {
        let path = path.into();
        self.parents(&path);
        self.tree
            .insert(path, Entry::File(bytes.into(), SystemTime::UNIX_EPOCH));
        self
    }

    #[must_use]
    pub fn modified(mut self, path: impl AsRef<Path>, time: SystemTime) -> Memory {
        if let Some(Entry::File(_, modified)) = self.tree.get_mut(path.as_ref()) {
            *modified = time;
        }
        self
    }

    #[must_use]
    pub fn clock(mut self, time: SystemTime) -> Memory {
        self.clock = Some(time);
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

    /// Every call on `path` fails with `PermissionDenied`, as below a folder that cannot be
    /// searched.
    #[must_use]
    pub fn denied(mut self, path: impl Into<PathBuf>) -> Memory {
        let path = path.into();
        self.parents(&path);
        self.tree.insert(path, Entry::Denied);
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
        match self.tree.get(path) {
            Some(Entry::Denied) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            Some(entry) => Ok(entry),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }
}

impl Files for Memory {
    fn now(&self) -> SystemTime {
        self.clock.unwrap_or_else(SystemTime::now)
    }

    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        if !path.is_absolute() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        self.entry(path).map(|_| path.to_path_buf())
    }

    fn node(&self, path: &Path) -> io::Result<Node> {
        Ok(match self.entry(path)? {
            Entry::File(..) => Node::File,
            Entry::Folder => Node::Folder,
            Entry::Other => Node::Other,
            Entry::Unreadable(node) => *node,
            Entry::Denied => unreachable!("entry() fails first"),
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
            Entry::File(bytes, _) => Ok(bytes.clone()),
            Entry::Unreadable(_) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }

    fn stamp(&self, path: &Path) -> io::Result<Stamp> {
        match self.entry(path)? {
            Entry::File(bytes, modified) => Ok(Stamp {
                size: bytes.len() as u64,
                modified: *modified,
            }),
            Entry::Unreadable(_) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }
}

/// The named call is still answered from `before`, every later call from `after`; `now` always
/// from `before`.
#[derive(Debug)]
pub struct Turning {
    before: Memory,
    after: Memory,
    at: (Call, PathBuf),
    turned: Cell<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Canonical,
    Node,
    List,
    Stamp,
    Read,
}

impl Turning {
    #[must_use]
    pub fn new(before: Memory, after: Memory, call: Call, path: impl Into<PathBuf>) -> Turning {
        Turning {
            before,
            after,
            at: (call, path.into()),
            turned: Cell::new(false),
        }
    }

    fn tree(&self, call: Call, path: &Path) -> &Memory {
        if self.turned.get() {
            return &self.after;
        }
        if self.at.0 == call && self.at.1 == path {
            self.turned.set(true);
        }
        &self.before
    }
}

impl Files for Turning {
    fn now(&self) -> SystemTime {
        self.before.now()
    }

    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        self.tree(Call::Canonical, path).canonical(path)
    }

    fn node(&self, path: &Path) -> io::Result<Node> {
        self.tree(Call::Node, path).node(path)
    }

    fn list(&self, folder: &Path) -> io::Result<Vec<OsString>> {
        self.tree(Call::List, folder).list(folder)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.tree(Call::Read, path).read(path)
    }

    fn stamp(&self, path: &Path) -> io::Result<Stamp> {
        self.tree(Call::Stamp, path).stamp(path)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use bilbo_store::entries::Stamp;

    use super::{Call, Files, Memory, Node, Turning};

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

    #[test]
    fn memory_stamps_a_file_with_its_size_and_the_time_it_was_given() {
        let later = SystemTime::UNIX_EPOCH + Duration::from_secs(9);
        let files = Memory::new()
            .file("/t/a.md", "abc")
            .file("/t/b.md", "b")
            .modified("/t/b.md", later)
            .modified("/t/none.md", later);
        let stamp = |path: &str| files.stamp(Path::new(path)).map_err(|e| e.kind());
        assert_eq!(
            stamp("/t/a.md"),
            Ok(Stamp {
                size: 3,
                modified: SystemTime::UNIX_EPOCH
            })
        );
        assert_eq!(
            stamp("/t/b.md"),
            Ok(Stamp {
                size: 1,
                modified: later
            })
        );
        assert_eq!(stamp("/t"), Err(ErrorKind::InvalidInput));
        assert_eq!(stamp("/t/none.md"), Err(ErrorKind::NotFound));
    }

    #[test]
    fn a_turning_tree_answers_from_the_second_tree_after_the_named_call() {
        let before = Memory::new()
            .file("/t/a", "1")
            .clock(SystemTime::UNIX_EPOCH);
        let after = Memory::new()
            .file("/t/b", "2")
            .clock(SystemTime::UNIX_EPOCH + Duration::from_secs(9));
        let files = Turning::new(before, after, Call::Stamp, "/t/a");

        assert_eq!(files.node(Path::new("/t/a")).ok(), Some(Node::File));
        assert!(files.stamp(Path::new("/t/a")).is_ok());
        assert_eq!(
            files.node(Path::new("/t/a")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );
        assert_eq!(files.node(Path::new("/t/b")).ok(), Some(Node::File));
        assert_eq!(
            files.canonical(Path::new("/t/a")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );
        assert_eq!(files.read(Path::new("/t/b")).ok(), Some(b"2".to_vec()));
        assert_eq!(files.now(), SystemTime::UNIX_EPOCH);
    }

    #[test]
    fn a_denied_path_fails_every_call_with_permission_denied() {
        let files = Memory::new().file("/t/a", "x").denied("/t/locked/b");
        let denied = Path::new("/t/locked/b");
        let kinds = [
            files.canonical(denied).map_err(|e| e.kind()).err(),
            files.node(denied).map_err(|e| e.kind()).err(),
            files.list(denied).map_err(|e| e.kind()).err(),
            files.read(denied).map_err(|e| e.kind()).err(),
            files.stamp(denied).map_err(|e| e.kind()).err(),
        ];
        assert_eq!(kinds, [Some(ErrorKind::PermissionDenied); 5]);
        assert_eq!(files.node(Path::new("/t/a")).ok(), Some(Node::File));
        assert_eq!(
            files.node(Path::new("/t/locked/c")).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );
    }
}
