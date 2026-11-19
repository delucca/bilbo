use std::fs::{File, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bilbo_derive::log::Log;
use bilbo_derive::plan::Due;
use bilbo_derive::spool::Spool;
use bilbo_derive::tries::{self, Extent, Tries};
use bilbo_store::id::Id;
use rustix::fs::{AtFlags, Dir, Mode, OFlags, mkdirat, openat, renameat, unlinkat};
use rustix::io::Errno;

use crate::console::private::{self, How};

const FOLDER: &str = "derive";
const LOCK: &str = "lock";
const SPOOL: &str = "spool";
const FAILED: &str = "failed";
const TEMPORARY: &str = "failed.tmp";
const LOG: &str = "log";
const BLOCK: &str = "block";
const PART: &str = "part";
const WORK: &str = "work";

pub type Sync = Rc<dyn Fn(&File, &Path) -> io::Result<()>>;

fn said(action: &str, path: &Path, error: &io::Error) -> io::Error {
    if private::is_refusal(error) {
        return io::Error::new(error.kind(), error.to_string());
    }
    io::Error::other(format!("cannot {action} {}: {error}", path.display()))
}

pub fn work(state: &Path) -> PathBuf {
    state.join(FOLDER).join(WORK)
}

fn flush(dir: &File, path: &Path) -> io::Result<()> {
    dir.sync_all().map_err(|error| said("flush", path, &error))
}

fn directory(parent: &File, name: &str) -> rustix::io::Result<File> {
    openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
}

fn existing(
    parent: &File,
    parent_path: &Path,
    name: &str,
    opening: &str,
) -> io::Result<Option<(File, PathBuf)>> {
    let path = parent_path.join(name);
    match directory(parent, name) {
        Ok(found) => Ok(Some((found, path))),
        Err(Errno::NOENT) => Ok(None),
        Err(error) => Err(said(opening, &path, &error.into())),
    }
}

fn made(
    parent: &File,
    parent_path: &Path,
    name: &str,
    opening: &str,
    sync: &Sync,
) -> io::Result<(File, PathBuf)> {
    made_by(parent, parent_path, name, opening, sync, || {
        mkdirat(parent, name, Mode::from_raw_mode(0o700))
    })
}

fn made_by(
    parent: &File,
    parent_path: &Path,
    name: &str,
    opening: &str,
    sync: &Sync,
    make: impl FnOnce() -> rustix::io::Result<()>,
) -> io::Result<(File, PathBuf)> {
    if let Some(found) = existing(parent, parent_path, name, opening)? {
        sync(parent, parent_path)?;
        return Ok(found);
    }
    let path = parent_path.join(name);
    match make() {
        Ok(()) | Err(Errno::EXIST) => sync(parent, parent_path)?,
        Err(error) => return Err(said("create", &path, &error.into())),
    }
    existing(parent, parent_path, name, opening)?
        .ok_or_else(|| said("create", &path, &io::Error::from(io::ErrorKind::NotFound)))
}

#[derive(Clone)]
pub struct Folder {
    state: PathBuf,
    folder: Rc<File>,
    sync: Sync,
}

impl Folder {
    pub fn at(state: &Path, folder: File) -> Folder {
        Folder::syncing(state, folder, Rc::new(flush))
    }

    fn syncing(state: &Path, folder: File, sync: Sync) -> Folder {
        Folder {
            state: state.to_owned(),
            folder: Rc::new(folder),
            sync,
        }
    }

    pub fn lock(&self) -> io::Result<Option<Held>> {
        let (derive, derive_path) = made(&self.folder, &self.state, FOLDER, "create", &self.sync)?;
        let (spool, spool_path) = made(&derive, &derive_path, SPOOL, "list", &self.sync)?;
        let lock_path = derive_path.join(LOCK);
        let lock = private::open(&derive, LOCK, &lock_path, How::Write)
            .map_err(|error| said("lock", &lock_path, &error))?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(Held {
                _lock: lock,
                derive,
                derive_path,
                spool,
                spool_path,
                sync: Rc::clone(&self.sync),
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(said("lock", &lock_path, &error)),
        }
    }

    pub fn charges(&self) -> io::Result<Failed> {
        Failed::read(
            existing(&self.folder, &self.state, FOLDER, "read")?,
            &self.sync,
        )
    }
}

pub struct Held {
    _lock: File,
    derive: File,
    derive_path: PathBuf,
    spool: File,
    spool_path: PathBuf,
    sync: Sync,
}

impl Held {
    pub fn blocks(&self) -> io::Result<Blocks> {
        let mut blocks = Blocks {
            spool: self.spool.try_clone()?,
            path: self.spool_path.clone(),
            sync: Rc::clone(&self.sync),
            next: 1,
        };
        let greatest = [BLOCK, PART]
            .into_iter()
            .map(|extension| blocks.names(extension))
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .filter_map(|name| name.split('.').next()?.parse::<u64>().ok())
            .max();
        blocks.next = match greatest {
            None => 1,
            Some(greatest) => greatest.checked_add(1).ok_or_else(|| {
                said(
                    "write",
                    &self.spool_path,
                    &io::Error::other("no number is left for a new block"),
                )
            })?,
        };
        Ok(blocks)
    }

    pub fn failed(&self) -> io::Result<Failed> {
        Failed::read(
            Some((self.derive.try_clone()?, self.derive_path.clone())),
            &self.sync,
        )
    }

    pub fn log(&self) -> io::Result<Calls> {
        Ok(Calls {
            derive: self.derive.try_clone()?,
            path: self.derive_path.join(LOG),
            error: None,
        })
    }
}

pub struct Blocks {
    spool: File,
    path: PathBuf,
    sync: Sync,
    next: u64,
}

impl Blocks {
    fn names(&self, extension: &str) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        let mut folder =
            Dir::read_from(&self.spool).map_err(|error| said("list", &self.path, &error.into()))?;
        while let Some(entry) = folder.read() {
            let entry = entry.map_err(|error| said("list", &self.path, &error.into()))?;
            let Ok(name) = entry.file_name().to_str() else {
                continue;
            };
            if Path::new(name)
                .extension()
                .is_some_and(|found| found == extension)
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.names(BLOCK)?.is_empty())
    }

    pub fn clear_parts(&self) -> io::Result<()> {
        for name in self.names(PART)? {
            let _ = unlinkat(&self.spool, &name, AtFlags::empty());
        }
        Ok(())
    }
}

impl Spool for Blocks {
    fn keep(&mut self, bytes: &[u8]) -> io::Result<()> {
        let number = self.next;
        let part = format!("{number:020}.{PART}");
        let part_path = self.path.join(&part);
        let mut file = private::open(&self.spool, &part, &part_path, How::New)
            .map_err(|error| said("write", &part_path, &error))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| said("write", &part_path, &error))?;
        let block = format!("{number:020}.{BLOCK}");
        renameat(&self.spool, &part, &self.spool, &block)
            .map_err(|error| said("rename", &part_path, &error.into()))?;
        (self.sync)(&self.spool, &self.path)?;
        self.next = number.checked_add(1).ok_or_else(|| {
            said(
                "write",
                &self.path,
                &io::Error::other("no number is left for a new block"),
            )
        })?;
        Ok(())
    }

    fn list(&self) -> io::Result<Vec<(String, Vec<u8>)>> {
        self.names(BLOCK)?
            .into_iter()
            .map(|name| {
                let path = self.path.join(&name);
                let mut bytes = Vec::new();
                private::open(&self.spool, &name, &path, How::Read)
                    .and_then(|mut file| file.read_to_end(&mut bytes))
                    .map_err(|error| said("read", &path, &error))?;
                Ok((name, bytes))
            })
            .collect()
    }

    fn remove(&mut self, name: &str) -> io::Result<()> {
        let path = self.path.join(name);
        unlinkat(&self.spool, name, AtFlags::empty())
            .map_err(|error| said("remove", &path, &error.into()))?;
        (self.sync)(&self.spool, &self.path)
    }
}

pub struct Failed {
    folder: Option<(File, PathBuf)>,
    lines: Vec<(Extent, u8)>,
    mark: Option<Id>,
    sync: Sync,
}

impl Failed {
    fn read(folder: Option<(File, PathBuf)>, sync: &Sync) -> io::Result<Failed> {
        let mut lines = Vec::new();
        let mut mark = None;
        if let Some((derive, derive_path)) = &folder {
            let path = derive_path.join(FAILED);
            let mut bytes = Vec::new();
            match private::open(derive, FAILED, &path, How::Read)
                .and_then(|mut file| file.read_to_end(&mut bytes))
            {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(said("read", &path, &error)),
            }
            for line in bytes.split(|byte| *byte == b'\n') {
                let Ok(line) = std::str::from_utf8(line) else {
                    continue;
                };
                if let Some(held) = tries::parse(line) {
                    lines.push(held);
                } else if let Some(entry) = tries::parse_mark(line) {
                    mark = Some(entry);
                }
            }
        }
        Ok(Failed {
            folder,
            lines,
            mark,
            sync: Rc::clone(sync),
        })
    }

    fn write(&self) -> io::Result<()> {
        let Some((derive, derive_path)) = &self.folder else {
            return Err(io::Error::other("no folder to write the charges in"));
        };
        let mut text = String::new();
        for (extent, charges) in &self.lines {
            text.push_str(&tries::line(extent, *charges));
            text.push('\n');
        }
        if let Some(entry) = self.mark {
            text.push_str(&tries::mark_line(entry));
            text.push('\n');
        }
        let path = derive_path.join(TEMPORARY);
        match unlinkat(derive, TEMPORARY, AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => {}
            Err(error) => return Err(said("remove", &path, &error.into())),
        }
        let mut file = private::open(derive, TEMPORARY, &path, How::New)
            .map_err(|error| said("write", &path, &error))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| said("write", &path, &error))?;
        renameat(derive, TEMPORARY, derive, FAILED)
            .map_err(|error| said("rename", &path, &error.into()))?;
        (self.sync)(derive, derive_path)
    }

    pub fn keep_only(&mut self, due: &[Due]) -> io::Result<()> {
        let before = self.lines.len();
        self.lines.retain(|(extent, _)| {
            let mine = due.iter().find(|entry| entry.parent.id == extent.parent);
            mine.is_none_or(|entry| {
                entry
                    .windows
                    .iter()
                    .any(|planned| planned.extent == *extent)
            })
        });
        if self.lines.len() == before {
            Ok(())
        } else {
            self.write()
        }
    }
}

impl Tries for Failed {
    fn charges(&self, extent: &Extent) -> io::Result<u8> {
        Ok(self
            .lines
            .iter()
            .find(|(held, _)| held == extent)
            .map_or(0, |(_, charges)| *charges))
    }

    fn charge(&mut self, extent: &Extent) -> io::Result<()> {
        match self.lines.iter_mut().find(|(held, _)| held == extent) {
            Some((_, charges)) => *charges = charges.saturating_add(1).min(tries::MOST),
            None => self.lines.push((*extent, 1)),
        }
        self.write()
    }

    fn forget(&mut self, extent: &Extent) -> io::Result<()> {
        self.lines.retain(|(held, _)| held != extent);
        self.write()
    }

    fn mark(&self) -> io::Result<Option<Id>> {
        Ok(self.mark)
    }

    fn set_mark(&mut self, entry: Option<Id>) -> io::Result<()> {
        if self.mark == entry {
            return Ok(());
        }
        self.mark = entry;
        self.write()
    }
}

pub struct Calls {
    derive: File,
    path: PathBuf,
    error: Option<String>,
}

impl Calls {
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn write(&self, line: &str) -> io::Result<()> {
        let mut file = private::open(&self.derive, LOG, &self.path, How::Append)?;
        file.write_all(format!("{line}\n").as_bytes())
    }
}

impl Log for Calls {
    fn append(&mut self, line: &str) {
        if self.error.is_none()
            && let Err(error) = self.write(line)
        {
            self.error = Some(said("write", &self.path, &error).to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs::{self, File};
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use bilbo_derive::log::Log;
    use bilbo_derive::spool::Spool;
    use bilbo_derive::tries::{Extent, Tries, line};
    use bilbo_store::entries::Generation;
    use bilbo_store::id::Id;

    use super::{Folder, Sync};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!(
                "disk-{test}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("a state folder");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("a mode");
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::set_permissions(
                self.0.join("derive/spool"),
                fs::Permissions::from_mode(0o700),
            );
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    type Seen = Rc<RefCell<Vec<(PathBuf, Vec<String>)>>>;

    fn recording() -> (Sync, Seen) {
        let seen: Seen = Rc::default();
        let sink = Rc::clone(&seen);
        let sync: Sync = Rc::new(move |_: &File, path: &Path| {
            let mut names: Vec<String> = fs::read_dir(path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            sink.borrow_mut().push((path.to_owned(), names));
            Ok(())
        });
        (sync, seen)
    }

    fn folder(scratch: &Scratch, sync: Sync) -> Folder {
        Folder::syncing(&scratch.0, File::open(&scratch.0).expect("open"), sync)
    }

    fn extent(first: u64) -> Extent {
        Extent {
            parent: Id::from_bytes([3; 16]),
            generation: Generation(1),
            first,
            last: first + 4,
        }
    }

    fn places(seen: &Seen) -> Vec<PathBuf> {
        seen.borrow().iter().map(|(path, _)| path.clone()).collect()
    }

    #[test]
    fn a_new_folder_and_a_kept_block_are_synced_into_their_parents() {
        let scratch = Scratch::new("parents");
        let (sync, seen) = recording();
        let state = &scratch.0;
        let held = folder(&scratch, Rc::clone(&sync))
            .lock()
            .expect("a lock")
            .expect("held");
        assert_eq!(places(&seen), [state.clone(), state.join("derive")]);
        assert!(folder(&scratch, sync).lock().expect("a lock").is_none());
        assert_eq!(seen.borrow().len(), 4);
        let mut blocks = held.blocks().expect("a spool");
        blocks.keep(b"a block").expect("kept");
        let seen = seen.borrow();
        assert_eq!(seen.len(), 5);
        assert_eq!(seen[4].0, state.join("derive/spool"));
        assert_eq!(seen[4].1, ["00000000000000000001.block"]);
    }

    #[test]
    fn a_folder_found_is_synced_into_its_parent_before_a_call_is_accepted() {
        let scratch = Scratch::new("found");
        let (recorder, seen) = recording();
        let calls = Rc::new(std::cell::Cell::new(0));
        let counted = Rc::clone(&calls);
        let sync: Sync = Rc::new(move |dir: &File, path: &Path| {
            counted.set(counted.get() + 1);
            if counted.get() == 2 {
                return Err(std::io::Error::other("a sync that fails"));
            }
            recorder(dir, path)
        });
        let first = folder(&scratch, Rc::clone(&sync)).lock();
        assert!(first.is_err());
        assert!(scratch.0.join("derive/spool").exists());
        let held = folder(&scratch, sync)
            .lock()
            .expect("a lock")
            .expect("held");
        let places = places(&seen);
        assert_eq!(
            places,
            [
                scratch.0.clone(),
                scratch.0.clone(),
                scratch.0.join("derive")
            ]
        );
        drop(held);
    }

    #[test]
    fn a_folder_another_run_made_first_is_synced_into_its_parent_too() {
        let scratch = Scratch::new("raced");
        let (sync, seen) = recording();
        let parent = File::open(&scratch.0).expect("a parent");
        let path = scratch.0.join("derive");
        let (_, found) = super::made_by(&parent, &scratch.0, "derive", "create", &sync, || {
            fs::create_dir(&path).expect("made by another run");
            Err(rustix::io::Errno::EXIST)
        })
        .expect("the folder");
        assert_eq!(found, path);
        assert_eq!(places(&seen), std::slice::from_ref(&scratch.0));
    }

    #[test]
    fn an_error_of_the_log_names_its_action_and_its_path() {
        let scratch = Scratch::new("log");
        let (sync, _) = recording();
        let held = folder(&scratch, sync)
            .lock()
            .expect("a lock")
            .expect("held");
        let path = scratch.0.join("derive/log");
        fs::write(&path, "").expect("a log");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).expect("a mode");
        let mut calls = held.log().expect("a log");
        calls.append("a line");
        let said = calls.error().expect("an error");
        assert!(
            said.starts_with(&format!("cannot write {}: ", path.display())),
            "{said}"
        );
    }

    #[test]
    fn a_dry_run_that_cannot_open_the_folder_says_read() {
        let scratch = Scratch::new("dry");
        let (sync, _) = recording();
        fs::create_dir(scratch.0.join("derive")).expect("a folder");
        fs::set_permissions(scratch.0.join("derive"), fs::Permissions::from_mode(0o000))
            .expect("a mode");
        let said = folder(&scratch, sync)
            .charges()
            .err()
            .expect("an error")
            .to_string();
        fs::set_permissions(scratch.0.join("derive"), fs::Permissions::from_mode(0o700))
            .expect("a mode");
        assert_eq!(
            said,
            format!(
                "cannot read {}: Permission denied (os error 13)",
                scratch.0.join("derive").display()
            )
        );
    }

    #[test]
    fn the_mark_is_kept_with_the_charges() {
        let scratch = Scratch::new("mark");
        let (sync, _) = recording();
        let held = folder(&scratch, sync)
            .lock()
            .expect("a lock")
            .expect("held");
        let entry = Id::from_bytes([5; 16]);
        let mut failed = held.failed().expect("charges");
        assert_eq!(failed.mark().expect("a mark"), None);
        failed.set_mark(Some(entry)).expect("set");
        failed.charge(&extent(1)).expect("charged");
        let again = held.failed().expect("charges");
        assert_eq!(again.mark().expect("a mark"), Some(entry));
        assert_eq!(again.charges(&extent(1)).expect("one"), 1);
        failed.keep_only(&[]).expect("kept");
        assert_eq!(
            held.failed().expect("charges").mark().expect("a mark"),
            Some(entry)
        );
        failed.set_mark(None).expect("cleared");
        assert_eq!(
            held.failed().expect("charges").mark().expect("a mark"),
            None
        );
        assert_eq!(
            held.failed()
                .expect("charges")
                .charges(&extent(1))
                .expect("one"),
            1
        );
    }

    #[test]
    fn a_charge_is_written_renamed_and_its_folder_synced() {
        let scratch = Scratch::new("charge");
        let (sync, seen) = recording();
        let held = folder(&scratch, sync)
            .lock()
            .expect("a lock")
            .expect("held");
        let mut failed = held.failed().expect("charges");
        seen.borrow_mut().clear();
        failed.charge(&extent(1)).expect("charged");
        {
            let seen = seen.borrow();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, scratch.0.join("derive"));
            assert!(seen[0].1.contains(&"failed".to_owned()));
            assert!(!seen[0].1.contains(&"failed.tmp".to_owned()));
        }
        let text = fs::read_to_string(scratch.0.join("derive/failed")).expect("charges");
        assert_eq!(text, format!("{}\n", line(&extent(1), 1)));
        seen.borrow_mut().clear();
        failed.forget(&extent(1)).expect("forgotten");
        assert_eq!(seen.borrow().len(), 1);
        assert_eq!(
            fs::read_to_string(scratch.0.join("derive/failed")).expect("charges"),
            ""
        );
        assert!(!scratch.0.join("derive/failed.tmp").exists());
    }

    #[test]
    fn the_charges_temporary_is_made_anew_and_never_written_through_a_link() {
        let scratch = Scratch::new("temporary");
        let (sync, _) = recording();
        let held = folder(&scratch, sync)
            .lock()
            .expect("a lock")
            .expect("held");
        let outside = scratch.0.join("outside");
        fs::write(&outside, "kept").expect("a file");
        let temporary = scratch.0.join("derive/failed.tmp");
        symlink(&outside, &temporary).expect("a link");
        let mut failed = held.failed().expect("charges");
        failed.charge(&extent(1)).expect("charged");
        assert_eq!(fs::read_to_string(&outside).expect("a file"), "kept");
        assert_eq!(
            fs::read_to_string(scratch.0.join("derive/failed")).expect("charges"),
            format!("{}\n", line(&extent(1), 1))
        );
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).expect("a mode");
        fs::hard_link(&outside, &temporary).expect("a second name");
        failed.charge(&extent(6)).expect("charged");
        assert_eq!(fs::read_to_string(&outside).expect("a file"), "kept");
        let text = fs::read_to_string(scratch.0.join("derive/failed")).expect("charges");
        assert_eq!(
            text,
            format!("{}\n{}\n", line(&extent(1), 1), line(&extent(6), 1))
        );
    }

    #[test]
    fn only_a_missing_file_of_charges_is_empty() {
        let scratch = Scratch::new("missing");
        let (sync, _) = recording();
        let held = folder(&scratch, sync)
            .lock()
            .expect("a lock")
            .expect("held");
        assert_eq!(
            held.failed()
                .expect("charges")
                .charges(&extent(1))
                .expect("none"),
            0
        );
        let mut bytes = format!("{}\n", line(&extent(1), 2)).into_bytes();
        bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
        bytes.extend_from_slice(b"not a line\n");
        bytes.extend_from_slice(format!("{}\n", line(&extent(6), 3)).as_bytes());
        let path = scratch.0.join("derive/failed");
        fs::write(&path, bytes).expect("charges");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("a mode");
        let failed = held.failed().expect("charges");
        assert_eq!(failed.charges(&extent(1)).expect("two"), 2);
        assert_eq!(failed.charges(&extent(6)).expect("three"), 3);
        assert_eq!(failed.charges(&extent(11)).expect("none"), 0);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("a mode");
        let error = held.failed().err().expect("an error");
        assert_eq!(
            error.to_string().split(": ").next(),
            Some(format!("cannot read {}", path.display()).as_str())
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("a mode");
    }

    #[test]
    fn no_block_is_numbered_past_the_greatest_number() {
        let scratch = Scratch::new("numbers");
        let (sync, _) = recording();
        let held = folder(&scratch, Rc::clone(&sync))
            .lock()
            .expect("a lock")
            .expect("held");
        let spool = scratch.0.join("derive/spool");
        let greatest = spool.join("18446744073709551615.block");
        fs::write(&greatest, "x").expect("a block");
        let error = held.blocks().err().expect("an error");
        assert_eq!(
            error.to_string(),
            format!(
                "cannot write {}: no number is left for a new block",
                spool.display()
            )
        );
        assert_eq!(fs::read_dir(&spool).expect("a folder").count(), 1);
        fs::remove_file(&greatest).expect("removed");
        for number in [3, 7] {
            fs::write(spool.join(format!("{number:020}.block")), "x").expect("a block");
        }
        let mut blocks = held.blocks().expect("a spool");
        fs::set_permissions(&spool, fs::Permissions::from_mode(0o300)).expect("a mode");
        blocks.keep(b"one").expect("kept");
        blocks.keep(b"two").expect("kept");
        fs::set_permissions(&spool, fs::Permissions::from_mode(0o700)).expect("a mode");
        let mut names: Vec<String> = fs::read_dir(&spool)
            .expect("a folder")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "00000000000000000003.block",
                "00000000000000000007.block",
                "00000000000000000008.block",
                "00000000000000000009.block",
            ]
        );
    }
}
