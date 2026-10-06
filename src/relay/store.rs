//! The relay's data folder: the transport tree at the paths a `file://` transport uses, `.relay.lock` held for the
//! relay's lifetime, and the durable create-only writer, which streams a body to `.tmp/`, flushes it, links it into
//! place only when the name is free, and flushes the folder before the relay answers.

use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::identity::keys;
use crate::shared::hash::Hasher;
use crate::sync::transport;

/// The mode of every folder the relay creates.
const FOLDER_MODE: u32 = 0o700;

/// The mode of a file the relay creates.
const FILE_MODE: u32 = 0o600;

/// The longest nameplate and message name of the layout.
const NAMEPLATE_MAX: usize = 64;
const MESSAGE_MAX: usize = 16;

/// What a create is about to do when it calls the hook of a test (`Data::step`, a no-op outside tests). A hook that fails stops the create at that step with
/// its error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Copy a chunk of the body to `.tmp/`.
    Write,
    /// Flush the temporary file.
    SyncFile,
    /// Create a missing folder of the path.
    MakeFolder,
    /// Flush a new folder, then its parent.
    SyncFolder,
    /// Link the temporary file to its name.
    Link,
    /// Flush the folder that holds the name.
    SyncParent,
}

#[cfg(test)]
type Hook = std::sync::Mutex<Box<dyn FnMut(Step) -> io::Result<()> + Send>>;

/// The data folder, locked while this value lives.
pub struct Data {
    root: PathBuf,
    _lock: File,
    #[cfg(test)]
    hook: Hook,
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
        match fs::metadata(root) {
            Ok(meta) if !meta.is_dir() => {
                return Err(format!("{} is not a folder", root.display()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                make_root(root)?;
            }
            Err(e) => return Err(format!("cannot read {}: {e}", root.display())),
        }
        let lock_path = root.join(".relay.lock");
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .mode(FILE_MODE)
            .open(&lock_path)
            .map_err(|e| format!("cannot open {}: {e}", lock_path.display()))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(format!("another relay serves {}", root.display()));
            }
            Err(TryLockError::Error(e)) => {
                return Err(format!("cannot lock {}: {e}", lock_path.display()));
            }
        }
        let tmp = root.join(".tmp");
        match fs::remove_dir_all(&tmp) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(format!("cannot empty {}: {e}", tmp.display()));
            }
            _ => {}
        }
        DirBuilder::new()
            .mode(FOLDER_MODE)
            .create(&tmp)
            .map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
        Ok(Data {
            root: root.to_path_buf(),
            _lock: lock,
            #[cfg(test)]
            hook: std::sync::Mutex::new(Box::new(|_| Ok(()))),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Streams `body`, which must yield exactly `length` bytes, to a new file under `.tmp/`, and flushes it. A failure
    /// removes the file and is `Full` or `Failed`.
    pub fn stage(&self, length: u64, body: &mut dyn Read) -> Result<Staged, Created> {
        let name = keys::random::<8>()
            .map(|bytes| keys::hex(&bytes))
            .map_err(Created::Failed)?;
        let path = self.root.join(".tmp").join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&path)
            .map_err(|e| failure(&path, "create", e))?;
        let mut staged = Staged {
            path,
            length,
            sha256: String::new(),
        };
        let mut hasher = Hasher::default();
        let mut left = length;
        let mut chunk = [0u8; 64 * 1024];
        while left > 0 {
            let want = chunk.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            let got = match body.read(&mut chunk[..want]) {
                Ok(0) => {
                    return Err(Created::Failed(format!(
                        "the body ended {left} bytes short of {length}"
                    )));
                }
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(failure(&staged.path, "read the body for", e)),
            };
            self.step(Step::Write)
                .and_then(|()| file.write_all(&chunk[..got]))
                .map_err(|e| failure(&staged.path, "write", e))?;
            hasher.update(&chunk[..got]);
            left -= got as u64;
        }
        self.step(Step::SyncFile)
            .and_then(|()| file.sync_all())
            .map_err(|e| failure(&staged.path, "flush", e))?;
        staged.sha256 = hasher.hex();
        Ok(staged)
    }

    /// The bytes at `path`, a path of the transport layout; `None` when nothing is there.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        if !is_layout(path) {
            return Err(format!("{path} is not in the layout"));
        }
        let file = self.root.join(path);
        match fs::read(&file) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", file.display())),
        }
    }

    /// Removes `pair/<nameplate>/` and what it holds.
    pub fn remove_nameplate(&self, nameplate: &str) -> Result<(), String> {
        if !transport::is_mailbox_name(nameplate, NAMEPLATE_MAX) {
            return Err(format!("{nameplate} is not a nameplate"));
        }
        let dir = self.root.join("pair").join(nameplate);
        match fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                Err(format!("cannot remove {}: {e}", dir.display()))
            }
            _ => Ok(()),
        }
    }

    #[cfg(test)]
    fn step(&self, step: Step) -> io::Result<()> {
        (self.hook.lock().unwrap())(step)
    }

    #[cfg(not(test))]
    fn step(&self, _: Step) -> io::Result<()> {
        Ok(())
    }

    /// Flushes the folder `dir`.
    fn sync_folder(&self, step: Step, dir: &Path) -> io::Result<()> {
        self.step(step)?;
        File::open(dir)?.sync_all()
    }

    /// Creates each missing folder of `parent` under the root and flushes it with its parent.
    fn make_folders(&self, parent: &str) -> io::Result<()> {
        let mut dir = self.root.clone();
        for part in parent.split('/') {
            let up = dir.clone();
            dir.push(part);
            self.step(Step::MakeFolder)?;
            match DirBuilder::new().mode(FOLDER_MODE).create(&dir) {
                Ok(()) => {
                    self.sync_folder(Step::SyncFolder, &dir)?;
                    self.sync_folder(Step::SyncFolder, &up)?;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
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
    /// is taken (`Same` or `Other` by its bytes, with nothing flushed), and flushes the parent before it returns `New`.
    pub fn link(self, data: &Data, path: &str) -> Created {
        if !is_layout(path) {
            return Created::Failed(format!("{path} is not in the layout"));
        }
        let target = data.root.join(path);
        let result = self.link_to(data, path, &target);
        result.unwrap_or_else(|e| failure(&target, "link", e))
    }

    fn link_to(&self, data: &Data, path: &str, target: &Path) -> io::Result<Created> {
        let parent = path.rsplit_once('/').map_or("", |(folder, _)| folder);
        data.make_folders(parent)?;
        let folder = target.parent().unwrap_or(&data.root);
        data.step(Step::Link)?;
        match fs::hard_link(&self.path, target) {
            Ok(()) => {
                data.sync_folder(Step::SyncParent, folder)?;
                Ok(Created::New)
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                Ok(if fs::read(target)? == self.bytes()? {
                    Created::Same
                } else {
                    Created::Other
                })
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Creates `root` and every missing folder above it with mode 0700, top down, flushing each with its parent.
fn make_root(root: &Path) -> Result<(), String> {
    let mut missing: Vec<&Path> = root
        .ancestors()
        .take_while(|dir| !dir.as_os_str().is_empty() && !dir.exists())
        .collect();
    missing.reverse();
    for dir in missing {
        let made = || -> io::Result<()> {
            DirBuilder::new().mode(FOLDER_MODE).create(dir)?;
            File::open(dir)?.sync_all()?;
            let up = dir.parent().filter(|up| !up.as_os_str().is_empty());
            File::open(up.unwrap_or(Path::new(".")))?.sync_all()
        };
        made().map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// Whether `path` is an object of the transport layout.
fn is_layout(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["scopes", scope, "manifest", name] => {
            keys::is_id(scope) && transport::manifest_number(name).is_some()
        }
        ["scopes", scope, "devices", device, name] => {
            keys::is_id(scope) && keys::is_id(device) && transport::segment_seq(name).is_some()
        }
        ["pair", nameplate, name] => {
            transport::is_mailbox_name(nameplate, NAMEPLATE_MAX)
                && name
                    .strip_suffix(".msg")
                    .is_some_and(|msg| transport::is_mailbox_name(msg, MESSAGE_MAX))
        }
        _ => false,
    }
}

/// `Full` for a disk or a quota that is full, `Failed` for anything else.
fn failure(path: &Path, what: &str, error: io::Error) -> Created {
    let message = format!("cannot {what} {}: {error}", path.display());
    match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => Created::Full(message),
        _ => Created::Failed(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Scratch(dir)
    }

    const SCOPE: &str = "234567abcdefghijklmnopqrst";
    const DEVICE: &str = "abcdefghijklmnopqrstuvwxyz";
    const MANIFEST: &str = "scopes/234567abcdefghijklmnopqrst/manifest/1.json";
    const SEGMENT: &str = "scopes/234567abcdefghijklmnopqrst/devices/abcdefghijklmnopqrstuvwxyz/00000000000000000001.seg";

    fn open(d: &Scratch) -> Data {
        Data::open(&d.0).unwrap()
    }

    fn stage(data: &Data, body: &[u8]) -> Staged {
        data.stage(body.len() as u64, &mut &body[..]).unwrap()
    }

    fn tmp_is_empty(data: &Data) -> bool {
        fs::read_dir(data.root().join(".tmp")).unwrap().count() == 0
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// Fails the `nth` call (from 1) of `at` with `error`, and records every step.
    fn fail_at(data: &Data, at: Step, nth: usize, kind: io::ErrorKind) -> Arc<Mutex<Vec<Step>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let mut count = 0;
        *data.hook.lock().unwrap() = Box::new(move |step| {
            log.lock().unwrap().push(step);
            if step == at {
                count += 1;
                if count == nth {
                    return Err(io::Error::from(kind));
                }
            }
            Ok(())
        });
        seen
    }

    #[test]
    fn open_creates_the_folder_locks_it_and_empties_tmp() {
        let d = dir("open");
        let data = open(&d);
        assert_eq!(mode(&d.0), 0o700);
        assert_eq!(mode(&d.0.join(".tmp")), 0o700);
        assert_eq!(
            Data::open(&d.0).err(),
            Some(format!("another relay serves {}", d.0.display()))
        );
        fs::write(d.0.join(".tmp/left"), b"x").unwrap();
        drop(data);
        let data = open(&d);
        assert!(tmp_is_empty(&data));
    }

    #[test]
    fn open_refuses_a_file() {
        let d = dir("file");
        fs::write(&d.0, b"x").unwrap();
        assert_eq!(
            Data::open(&d.0).err(),
            Some(format!("{} is not a folder", d.0.display()))
        );
        fs::remove_file(&d.0).unwrap();
    }

    #[test]
    fn stage_hashes_and_counts_the_body() {
        let d = dir("stage");
        let data = open(&d);
        let body = vec![7u8; 200_000];
        let staged = stage(&data, &body);
        assert_eq!(staged.length(), 200_000);
        assert_eq!(staged.sha256_hex(), crate::shared::hash::sha256_hex(&body));
        assert_eq!(staged.bytes().unwrap(), body);
        drop(staged);
        assert!(tmp_is_empty(&data));
    }

    #[test]
    fn a_short_body_is_removed() {
        let d = dir("short");
        let data = open(&d);
        let got = data.stage(10, &mut &b"abc"[..]);
        assert!(matches!(got, Err(Created::Failed(_))));
        assert!(tmp_is_empty(&data));
    }

    #[test]
    fn stage_reads_no_more_than_the_length() {
        let d = dir("long");
        let data = open(&d);
        let mut body = &b"abcdef"[..];
        let staged = data.stage(3, &mut body).unwrap();
        assert_eq!(staged.bytes().unwrap(), b"abc");
        assert_eq!(body, b"def");
    }

    #[test]
    fn link_creates_folders_and_answers_by_bytes() {
        let d = dir("link");
        let data = open(&d);
        assert_eq!(stage(&data, b"one").link(&data, MANIFEST), Created::New);
        assert_eq!(fs::read(d.0.join(MANIFEST)).unwrap(), b"one");
        assert_eq!(mode(&d.0.join("scopes")), 0o700);
        assert_eq!(
            mode(&d.0.join("scopes").join(SCOPE).join("manifest")),
            0o700
        );
        assert_eq!(stage(&data, b"one").link(&data, MANIFEST), Created::Same);
        assert_eq!(stage(&data, b"two").link(&data, MANIFEST), Created::Other);
        assert_eq!(fs::read(d.0.join(MANIFEST)).unwrap(), b"one");
        assert!(tmp_is_empty(&data));
        assert_eq!(data.read(MANIFEST).unwrap(), Some(b"one".to_vec()));
        assert_eq!(
            data.read("scopes/234567abcdefghijklmnopqrst/manifest/2.json"),
            Ok(None)
        );
    }

    #[test]
    fn a_create_runs_its_steps_in_order() {
        let d = dir("order");
        let data = open(&d);
        let seen = fail_at(&data, Step::Link, usize::MAX, io::ErrorKind::Other);
        assert_eq!(stage(&data, b"x").link(&data, SEGMENT), Created::New);
        let mut want = vec![Step::Write, Step::SyncFile];
        for _ in 0..4 {
            want.extend([Step::MakeFolder, Step::SyncFolder, Step::SyncFolder]);
        }
        want.extend([Step::Link, Step::SyncParent]);
        assert_eq!(*seen.lock().unwrap(), want);
    }

    #[test]
    fn an_existing_folder_is_not_flushed_again() {
        let d = dir("again");
        let data = open(&d);
        assert_eq!(stage(&data, b"x").link(&data, MANIFEST), Created::New);
        let seen = fail_at(&data, Step::Link, usize::MAX, io::ErrorKind::Other);
        let other = "scopes/234567abcdefghijklmnopqrst/manifest/2.json";
        assert_eq!(stage(&data, b"y").link(&data, other), Created::New);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.iter().filter(|s| **s == Step::SyncFolder).count(), 0);
        assert_eq!(seen.last(), Some(&Step::SyncParent));
    }

    /// Every failing step leaves nothing under `.tmp/` and no object, and `Full` kinds are told apart.
    #[test]
    fn a_taken_name_flushes_nothing() {
        let d = dir("taken");
        let data = open(&d);
        assert_eq!(stage(&data, b"body").link(&data, SEGMENT), Created::New);
        let seen = fail_at(&data, Step::SyncParent, 1, io::ErrorKind::PermissionDenied);
        assert_eq!(stage(&data, b"body").link(&data, SEGMENT), Created::Same);
        assert_eq!(stage(&data, b"other").link(&data, SEGMENT), Created::Other);
        assert!(!seen.lock().unwrap().contains(&Step::SyncParent));
    }

    #[test]
    fn a_missing_data_folder_is_made_with_each_missing_parent_at_0700() {
        let d = dir("nested");
        let root = d.0.join("a").join("b").join("data");
        drop(Data::open(&root).unwrap());
        for folder in [d.0.join("a"), d.0.join("a").join("b"), root] {
            let mode = fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", folder.display());
        }
    }

    #[test]
    fn a_failure_at_any_step_cleans_up() {
        let steps = [
            (Step::Write, 1),
            (Step::SyncFile, 1),
            (Step::MakeFolder, 1),
            (Step::MakeFolder, 4),
            (Step::SyncFolder, 1),
            (Step::SyncFolder, 2),
            (Step::SyncFolder, 8),
            (Step::Link, 1),
            (Step::SyncParent, 1),
        ];
        for (step, nth) in steps {
            for (kind, full) in [
                (io::ErrorKind::PermissionDenied, false),
                (io::ErrorKind::StorageFull, true),
                (io::ErrorKind::QuotaExceeded, true),
            ] {
                let d = dir("fail");
                let data = open(&d);
                fail_at(&data, step, nth, kind);
                let body = b"body";
                let got = match data.stage(4, &mut &body[..]) {
                    Ok(staged) => staged.link(&data, SEGMENT),
                    Err(created) => created,
                };
                let label = format!("{step:?} #{nth} {kind:?}: {got:?}");
                match got {
                    Created::Full(_) => assert!(full, "{label}"),
                    Created::Failed(_) => assert!(!full, "{label}"),
                    _ => panic!("{label}"),
                }
                assert!(tmp_is_empty(&data), "{label}");
                if step != Step::SyncParent {
                    assert!(!d.0.join(SEGMENT).exists(), "{label}");
                }
            }
        }
    }

    #[test]
    fn a_retry_after_a_failure_is_new() {
        let d = dir("retry");
        let data = open(&d);
        fail_at(&data, Step::SyncFolder, 3, io::ErrorKind::StorageFull);
        assert!(matches!(
            stage(&data, b"x").link(&data, MANIFEST),
            Created::Full(_)
        ));
        fail_at(&data, Step::Link, usize::MAX, io::ErrorKind::Other);
        assert_eq!(stage(&data, b"x").link(&data, MANIFEST), Created::New);
    }

    #[test]
    fn paths_outside_the_layout_are_refused() {
        let d = dir("layout");
        let data = open(&d);
        for path in [
            "",
            "scopes",
            "scopes/x/manifest/1.json",
            "scopes/234567abcdefghijklmnopqrst/manifest/01.json",
            "scopes/234567abcdefghijklmnopqrst/manifest/../1.json",
            "scopes/234567abcdefghijklmnopqrst/devices/abcdefghijklmnopqrstuvwxyz/1.seg",
            "pair/../a.msg",
            "pair/np/a.txt",
            "/etc/passwd",
            ".tmp/a",
        ] {
            assert!(data.read(path).is_err(), "{path}");
            assert!(
                matches!(stage(&data, b"x").link(&data, path), Created::Failed(_)),
                "{path}"
            );
        }
        assert!(tmp_is_empty(&data));
        assert_eq!(
            stage(&data, b"x").link(&data, "pair/np-1/a.msg"),
            Created::New
        );
    }

    #[test]
    fn remove_nameplate_removes_the_folder() {
        let d = dir("remove");
        let data = open(&d);
        assert_eq!(
            stage(&data, b"x").link(&data, "pair/np/a.msg"),
            Created::New
        );
        assert_eq!(
            stage(&data, b"y").link(&data, "pair/np/b.msg"),
            Created::New
        );
        data.remove_nameplate("np").unwrap();
        assert!(!d.0.join("pair/np").exists());
        data.remove_nameplate("np").unwrap();
        assert!(data.remove_nameplate("../x").is_err());
        assert!(d.0.join("pair").exists());
    }

    #[test]
    fn nothing_is_left_when_the_staged_body_is_dropped() {
        let d = dir("drop");
        let data = open(&d);
        let staged = stage(&data, b"x");
        assert!(!tmp_is_empty(&data));
        drop(staged);
        assert!(tmp_is_empty(&data));
        let _ = SCOPE;
        let _ = DEVICE;
    }
}
