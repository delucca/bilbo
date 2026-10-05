//! The transport interface, picked by URL scheme, and its `file://` folder: create-only objects, listings and the
//! pairing mailbox.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::host::swap;
use crate::identity::keys;

/// The largest object `get` reads: a segment is at most 8 MiB and a manifest is far smaller.
pub const OBJECT_MAX: u64 = 16 * 1024 * 1024;

/// How old one of this device's temporary files must be before a sweep removes it.
const TEMPORARY_AGE: Duration = Duration::from_secs(60 * 60);

/// The result of creating an object.
#[derive(Debug, PartialEq)]
pub enum Put {
    Created,
    Exists,
    /// The transport's own message for a full disk or an exhausted quota.
    Full(String),
    /// The transport could not be reached, with the reason.
    Unreachable(String),
}

/// A place devices leave each other objects, named by relative paths of the layout.
pub trait Transport: Send {
    /// The reason the transport cannot be reached at all, such as a root that is not there.
    fn reachable(&self) -> Result<(), String>;

    /// Whether the transport keeps what it stores, so a deleted object is never created again: `false` for a folder.
    fn keeps(&self) -> bool;

    /// The scope ids under `scopes/`, sorted.
    fn scopes(&self) -> Result<Vec<String>, String>;

    /// The device ids that have a folder in `scope`, sorted.
    fn devices(&self, scope: &str) -> Result<Vec<String>, String>;

    /// The seqs above `cursor` that `device`'s folder lists, sorted.
    fn list_after(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String>;

    /// The seqs `cursor + 1`, `cursor + 2`, … that exist, up to the first that does not.
    fn probe(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String>;

    /// The bytes of an object, `None` when it is not there or `path` is not in the layout.
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String>;

    /// Creates an object that does not exist.
    fn create(&self, path: &str, bytes: &[u8]) -> Put;

    /// The highest manifest number of `scope`, `None` when it has none.
    fn highest_manifest(&self, scope: &str) -> Result<Option<u64>, String>;

    /// Replaces a damaged segment of this device's own folder. Only a transport that does not keep what it stores
    /// does it.
    fn replace(&self, path: &str, bytes: &[u8]) -> Result<(), String>;

    /// Removes this device's temporary files older than an hour at `now`.
    fn sweep(&self, now: SystemTime) -> Result<(), String>;

    /// Removes `pair/<nameplate>/`, the one deletion the layout allows. Pairing (change 5) calls it.
    #[cfg_attr(not(test), expect(dead_code))]
    fn remove_mailbox(&self, nameplate: &str) -> Result<(), String>;
}

/// The transport for `url`, for the device `device`.
pub fn open(url: &str, device: &str) -> Result<Box<dyn Transport>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return if path.starts_with('/') {
            Ok(Box::new(Folder::new(PathBuf::from(path), device)))
        } else {
            Err(format!("{url} is not an absolute path"))
        };
    }
    let scheme = url.split("://").next().unwrap_or(url);
    Err(format!(
        "{scheme} transports are not supported yet; use a file:// folder"
    ))
}

pub fn manifest_path(scope: &str, n: u64) -> String {
    format!("scopes/{scope}/manifest/{n}.json")
}

pub fn segment_path(scope: &str, device: &str, seq: u64) -> String {
    format!("scopes/{scope}/devices/{device}/{seq:020}.seg")
}

#[cfg_attr(not(test), expect(dead_code))]
pub fn message_path(nameplate: &str, msg: &str) -> String {
    format!("pair/{nameplate}/{msg}.msg")
}

/// An object named by a path of the layout.
enum Object<'a> {
    Manifest,
    /// A segment, with its device.
    Segment(&'a str),
    Message,
}

/// The folder of an object, relative to the root.
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// A 26-character base32 id.
fn is_id(text: &str) -> bool {
    text.len() == 26
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// A mailbox name: 1 to `max` characters of `[a-z0-9-]`.
fn is_mailbox_name(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A manifest number as its file name spells it: decimal from 1, no leading zero.
fn manifest_number(name: &str) -> Option<u64> {
    let n = name.strip_suffix(".json")?.parse::<u64>().ok()?;
    (n > 0 && format!("{n}.json") == name).then_some(n)
}

/// A seq as its file name spells it: 20 digits from 1.
fn segment_seq(name: &str) -> Option<u64> {
    let digits = name.strip_suffix(".seg")?;
    if digits.len() != 20 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok().filter(|seq| *seq > 0)
}

/// The layout's object `path` names, `None` for any other path.
fn parse(path: &str) -> Option<Object<'_>> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["scopes", scope, "manifest", name] if is_id(scope) => {
            manifest_number(name).map(|_| Object::Manifest)
        }
        ["scopes", scope, "devices", device, name] if is_id(scope) && is_id(device) => {
            segment_seq(name).map(|_| Object::Segment(device))
        }
        ["pair", nameplate, name] if is_mailbox_name(nameplate, 64) => name
            .strip_suffix(".msg")
            .filter(|msg| is_mailbox_name(msg, 16))
            .map(|_| Object::Message),
        _ => None,
    }
}

/// Whether `name` is a temporary file of `device`: `.<device id>-<16 lowercase hexadecimal characters>.tmp`.
fn is_temporary(name: &str, device: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|rest| rest.strip_prefix(device))
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.strip_suffix(".tmp"))
        .is_some_and(|hex| {
            hex.len() == 16
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

/// The `file://` transport: the tree under a folder that must already exist.
pub struct Folder {
    root: PathBuf,
    device: String,
}

impl Folder {
    pub fn new(root: PathBuf, device: &str) -> Folder {
        Folder {
            root,
            device: device.to_string(),
        }
    }

    /// The reason the root cannot be used, when it cannot: an unmounted volume must not read as an empty one.
    fn check_root(&self) -> Result<(), String> {
        match fs::metadata(&self.root) {
            Ok(meta) if meta.is_dir() => Ok(()),
            Ok(_) => Err("it is not a folder".into()),
            Err(e) => Err(reason(&e)),
        }
    }

    /// The names of the entries of `relative` for which `keep` holds, sorted; none when the folder is missing.
    fn names(
        &self,
        relative: &str,
        keep: impl Fn(&fs::FileType) -> bool,
    ) -> Result<Vec<String>, String> {
        self.check_root()?;
        let dir = self.root.join(relative);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if keep(&kind)
                && let Ok(name) = entry.file_name().into_string()
            {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn seqs(&self, scope: &str, device: &str) -> Result<Vec<u64>, String> {
        ids(&[scope, device])?;
        let names = self.names(&format!("scopes/{scope}/devices/{device}"), |k| k.is_file())?;
        let mut seqs: Vec<u64> = names.iter().filter_map(|n| segment_seq(n)).collect();
        seqs.sort_unstable();
        Ok(seqs)
    }

    /// Creates `relative` below the root one component at a time, so the root itself is never created, and returns
    /// its path. A component that is a symbolic link is refused.
    fn make_folder(&self, relative: &str) -> io::Result<PathBuf> {
        let mut path = self.root.clone();
        for part in relative.split('/').filter(|p| !p.is_empty()) {
            path.push(part);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => {
                    return Err(io::Error::other(format!(
                        "{} is not a folder",
                        path.display()
                    )));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    DirBuilder::new().mode(0o700).create(&path)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(path)
    }

    /// Writes `bytes` to a fresh hidden temporary in `folder` through `write`, and returns its path.
    fn stage(
        &self,
        folder: &Path,
        bytes: &[u8],
        write: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
    ) -> Result<PathBuf, Put> {
        let random = keys::random::<8>().map_err(Put::Unreachable)?;
        let temporary = folder.join(format!(".{}-{}.tmp", self.device, keys::hex(&random)));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| classify(&e))?;
        if let Err(e) = write(&mut file, bytes).and_then(|()| file.sync_all()) {
            let _ = fs::remove_file(&temporary);
            return Err(classify(&e));
        }
        Ok(temporary)
    }

    fn create_with(
        &self,
        path: &str,
        bytes: &[u8],
        write: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
    ) -> Put {
        if parse(path).is_none() {
            return Put::Unreachable(format!("{path} is not in the transport layout"));
        }
        if let Err(why) = self.check_root() {
            return Put::Unreachable(why);
        }
        let target = self.root.join(path);
        if target.symlink_metadata().is_ok() {
            return Put::Exists;
        }
        let folder = match self.make_folder(folder_of(path)) {
            Ok(folder) => folder,
            Err(e) => return classify(&e),
        };
        let temporary = match self.stage(&folder, bytes, write) {
            Ok(temporary) => temporary,
            Err(put) => return put,
        };
        if let Err(message) = swap::rename_new(&temporary, &target) {
            let _ = fs::remove_file(&temporary);
            return if target.symlink_metadata().is_ok() {
                Put::Exists
            } else if message.contains("No space left") || message.contains("quota") {
                Put::Full(message)
            } else {
                Put::Unreachable(message)
            };
        }
        sync_folder(&folder);
        Put::Created
    }
}

/// An error for an id that is not 26 base32 characters.
fn ids(ids: &[&str]) -> Result<(), String> {
    match ids.iter().find(|id| !is_id(id)) {
        Some(id) => Err(format!("{id} is not an id of the transport layout")),
        None => Ok(()),
    }
}

/// The reason an `io::Error` on the root gives.
fn reason(e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::NotFound {
        "the folder does not exist".into()
    } else {
        e.to_string()
    }
}

fn classify(e: &io::Error) -> Put {
    match e.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => Put::Full(e.to_string()),
        _ => Put::Unreachable(e.to_string()),
    }
}

fn sync_folder(folder: &Path) {
    if let Ok(dir) = File::open(folder) {
        let _ = dir.sync_all();
    }
}

fn write_all(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)
}

impl Transport for Folder {
    fn reachable(&self) -> Result<(), String> {
        self.check_root()
    }

    fn keeps(&self) -> bool {
        false
    }

    fn scopes(&self) -> Result<Vec<String>, String> {
        let names = self.names("scopes", |k| k.is_dir())?;
        Ok(names.into_iter().filter(|n| is_id(n)).collect())
    }

    fn devices(&self, scope: &str) -> Result<Vec<String>, String> {
        ids(&[scope])?;
        let names = self.names(&format!("scopes/{scope}/devices"), |k| k.is_dir())?;
        Ok(names.into_iter().filter(|n| is_id(n)).collect())
    }

    fn list_after(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
        let mut seqs = self.seqs(scope, device)?;
        seqs.retain(|seq| *seq > cursor);
        Ok(seqs)
    }

    fn probe(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
        ids(&[scope, device])?;
        self.check_root()?;
        let mut found = Vec::new();
        for seq in cursor.saturating_add(1).. {
            let path = self.root.join(segment_path(scope, device, seq));
            match path.symlink_metadata() {
                Ok(meta) if meta.is_file() => found.push(seq),
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::NotFound => break,
                Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
            }
        }
        Ok(found)
    }

    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        if parse(path).is_none() {
            return Err(format!("{path} is not in the transport layout"));
        }
        self.check_root()?;
        let file = self.root.join(path);
        match file.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("{} is a symbolic link", file.display()));
            }
            Ok(meta) if meta.len() > OBJECT_MAX => {
                return Err(format!(
                    "{} is {} bytes, over the {OBJECT_MAX} byte limit",
                    file.display(),
                    meta.len()
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
        }
        match fs::read(&file) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", file.display())),
        }
    }

    fn create(&self, path: &str, bytes: &[u8]) -> Put {
        self.create_with(path, bytes, &write_all)
    }

    fn highest_manifest(&self, scope: &str) -> Result<Option<u64>, String> {
        ids(&[scope])?;
        let names = self.names(&format!("scopes/{scope}/manifest"), |k| k.is_file())?;
        Ok(names.iter().filter_map(|n| manifest_number(n)).max())
    }

    fn replace(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        match parse(path) {
            Some(Object::Segment(device)) if device == self.device => {}
            _ => return Err(format!("{path} is not a segment of this device")),
        }
        self.check_root()?;
        let target = self.root.join(path);
        let folder = self
            .make_folder(folder_of(path))
            .map_err(|e| format!("cannot create {}: {e}", path))?;
        let temporary = self
            .stage(&folder, bytes, &write_all)
            .map_err(|put| match put {
                Put::Full(why) | Put::Unreachable(why) => {
                    format!("cannot write {}: {why}", target.display())
                }
                Put::Created | Put::Exists => {
                    unreachable!("staging fails only as full or unreachable")
                }
            })?;
        fs::rename(&temporary, &target).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("cannot replace {}: {e}", target.display())
        })?;
        sync_folder(&folder);
        Ok(())
    }

    fn sweep(&self, now: SystemTime) -> Result<(), String> {
        self.check_root()?;
        let mut folders = Vec::new();
        for nameplate in self.names("pair", |k| k.is_dir())? {
            if is_mailbox_name(&nameplate, 64) {
                folders.push(format!("pair/{nameplate}"));
            }
        }
        for scope in self.scopes()? {
            folders.push(format!("scopes/{scope}/manifest"));
            folders.push(format!("scopes/{scope}/devices/{}", self.device));
        }
        for folder in folders {
            for name in self.names(&folder, |k| k.is_file())? {
                if !is_temporary(&name, &self.device) {
                    continue;
                }
                let path = self.root.join(&folder).join(&name);
                let age = fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok());
                if age.is_some_and(|age| age >= TEMPORARY_AGE) {
                    match fs::remove_file(&path) {
                        Err(e) if e.kind() != io::ErrorKind::NotFound => {
                            return Err(format!("cannot remove {}: {e}", path.display()));
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn remove_mailbox(&self, nameplate: &str) -> Result<(), String> {
        if !is_mailbox_name(nameplate, 64) {
            return Err(format!("{nameplate} is not a nameplate"));
        }
        self.check_root()?;
        let dir = self.root.join("pair").join(nameplate);
        match fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                Err(format!("cannot remove {}: {e}", dir.display()))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("bilbo-transport-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    const A: &str = "abcdefghijklmnopqrstuvwxyz";
    const B: &str = "bcdefghijklmnopqrstuvwxyz2";
    const SCOPE: &str = "234567abcdefghijklmnopqrst";

    fn folder(d: &Scratch, device: &str) -> Folder {
        Folder::new(d.0.clone(), device)
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_first_segment_is_created_in_its_own_folder_with_private_modes() {
        let d = dir("first_segment");
        let t = folder(&d, A);
        let path = segment_path(SCOPE, A, 1);
        assert_eq!(
            path,
            format!("scopes/{SCOPE}/devices/{A}/00000000000000000001.seg")
        );
        assert_eq!(t.create(&path, b"one"), Put::Created);
        assert_eq!(fs::read(d.0.join(&path)).unwrap(), b"one");
        assert_eq!(mode(&d.0.join(&path)), 0o600);
        for sub in [
            "scopes",
            &format!("scopes/{SCOPE}"),
            &format!("scopes/{SCOPE}/devices"),
            &format!("scopes/{SCOPE}/devices/{A}"),
        ] {
            assert_eq!(mode(&d.0.join(sub)), 0o700, "{sub}");
        }
        assert_eq!(
            names(&d.0.join(format!("scopes/{SCOPE}/devices/{A}"))).len(),
            1
        );
        assert_eq!(t.get(&path).unwrap().unwrap(), b"one");
        assert_eq!(t.scopes().unwrap(), [SCOPE]);
        assert_eq!(t.devices(SCOPE).unwrap(), [A]);
    }

    #[test]
    fn creating_an_existing_object_reports_it_and_changes_nothing() {
        let d = dir("exists");
        let t = folder(&d, A);
        let path = manifest_path(SCOPE, 1);
        assert_eq!(t.create(&path, b"one"), Put::Created);
        assert_eq!(t.create(&path, b"one"), Put::Exists);
        assert_eq!(t.create(&path, b"two"), Put::Exists);
        assert_eq!(t.get(&path).unwrap().unwrap(), b"one");
        assert_eq!(
            names(&d.0.join(format!("scopes/{SCOPE}/manifest"))),
            ["1.json"]
        );
    }

    #[test]
    fn a_full_disk_is_reported_with_its_message_and_leaves_nothing_behind() {
        let d = dir("full");
        let t = folder(&d, A);
        let path = segment_path(SCOPE, A, 1);
        let full = |_: &mut File, _: &[u8]| Err(io::Error::from(io::ErrorKind::StorageFull));
        let Put::Full(message) = t.create_with(&path, b"one", &full) else {
            panic!("not full");
        };
        assert!(!message.is_empty());
        assert!(!d.0.join(&path).exists());
        assert!(names(&d.0.join(format!("scopes/{SCOPE}/devices/{A}"))).is_empty());
        assert_eq!(t.create(&path, b"one"), Put::Created);
    }

    #[test]
    fn a_missing_root_is_unreachable_and_never_created() {
        let d = dir("missing_root");
        let root = d.0.join("usb");
        let t = Folder::new(root.clone(), A);
        let Put::Unreachable(why) = t.create(&manifest_path(SCOPE, 1), b"one") else {
            panic!("created");
        };
        assert_eq!(why, "the folder does not exist");
        assert!(!root.exists());
        assert_eq!(t.scopes().unwrap_err(), why);
        assert!(t.get(&manifest_path(SCOPE, 1)).is_err());
        assert!(t.sweep(SystemTime::now()).is_err());
        assert!(t.remove_mailbox("7").is_err());
    }

    #[test]
    fn a_root_with_no_tree_lists_nothing() {
        let d = dir("empty_root");
        let t = folder(&d, A);
        assert!(t.scopes().unwrap().is_empty());
        assert!(t.devices(SCOPE).unwrap().is_empty());
        assert!(t.list_after(SCOPE, A, 0).unwrap().is_empty());
        assert!(t.probe(SCOPE, A, 0).unwrap().is_empty());
        assert_eq!(t.highest_manifest(SCOPE).unwrap(), None);
        assert_eq!(t.get(&manifest_path(SCOPE, 1)).unwrap(), None);
    }

    #[test]
    fn names_outside_the_layout_are_invisible() {
        let d = dir("conflict_copy");
        let t = folder(&d, A);
        for seq in [1, 2] {
            assert_eq!(t.create(&segment_path(SCOPE, B, seq), b"x"), Put::Created);
        }
        let devices = d.0.join(format!("scopes/{SCOPE}/devices/{B}"));
        for name in [
            "00000000000000000003 (conflicted copy).seg",
            "3.seg",
            "00000000000000000000.seg",
            "0000000000000000000A.seg",
            ".hidden.seg",
        ] {
            fs::write(devices.join(name), "x").unwrap();
        }
        fs::create_dir_all(d.0.join(format!("scopes/{SCOPE}/devices/not-a-device"))).unwrap();
        fs::create_dir_all(d.0.join("scopes/not-a-scope")).unwrap();
        fs::write(d.0.join("scopes/stray.json"), "x").unwrap();
        assert_eq!(t.list_after(SCOPE, B, 0).unwrap(), [1, 2]);
        assert_eq!(t.probe(SCOPE, B, 0).unwrap(), [1, 2]);
        assert_eq!(t.devices(SCOPE).unwrap(), [B]);
        assert_eq!(t.scopes().unwrap(), [SCOPE]);
        let manifests = d.0.join(format!("scopes/{SCOPE}/manifest"));
        fs::create_dir_all(manifests.join("lost")).unwrap();
        for name in [
            "2.json",
            "01.json",
            "3 (conflicted copy).json",
            ".4.json",
            "5.pending",
        ] {
            fs::write(manifests.join(name), "x").unwrap();
        }
        assert_eq!(t.highest_manifest(SCOPE).unwrap(), Some(2));
        assert!(t.get("scopes/stray.json").is_err());
    }

    #[test]
    fn a_poll_probes_past_the_cursor_and_a_listing_finds_what_a_gap_hides() {
        let d = dir("gap");
        let t = folder(&d, A);
        for seq in [1, 2, 4] {
            assert_eq!(t.create(&segment_path(SCOPE, B, seq), b"x"), Put::Created);
        }
        assert_eq!(t.probe(SCOPE, B, 0).unwrap(), [1, 2]);
        assert_eq!(t.probe(SCOPE, B, 2).unwrap(), Vec::<u64>::new());
        assert_eq!(t.probe(SCOPE, B, 3).unwrap(), [4]);
        assert_eq!(t.list_after(SCOPE, B, 1).unwrap(), [2, 4]);
        assert_eq!(t.list_after(SCOPE, B, 4).unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn mailbox_names_follow_the_layout() {
        let d = dir("mailbox");
        let t = folder(&d, A);
        assert_eq!(t.create(&message_path("7", "a"), b"hello"), Put::Created);
        fs::write(d.0.join("pair/7/A_1.msg"), "x").unwrap();
        assert_eq!(t.get("pair/7/a.msg").unwrap().unwrap(), b"hello");
        assert!(t.get("pair/7/A_1.msg").is_err());
        assert!(matches!(
            t.create("pair/7/A_1.msg", b"x"),
            Put::Unreachable(_)
        ));
        assert!(matches!(
            t.create("pair/7/../x.msg", b"x"),
            Put::Unreachable(_)
        ));
        let long = "a".repeat(65);
        assert!(matches!(
            t.create(&message_path(&long, "a"), b"x"),
            Put::Unreachable(_)
        ));
        assert!(matches!(
            t.create(&message_path("7", &"a".repeat(17)), b"x"),
            Put::Unreachable(_)
        ));
    }

    #[test]
    fn removing_a_mailbox_leaves_the_scopes_alone() {
        let d = dir("remove_mailbox");
        let t = folder(&d, A);
        assert_eq!(t.create(&message_path("7", "a"), b"x"), Put::Created);
        assert_eq!(t.create(&message_path("8", "a"), b"x"), Put::Created);
        assert_eq!(t.create(&manifest_path(SCOPE, 1), b"m"), Put::Created);
        t.remove_mailbox("7").unwrap();
        assert!(!d.0.join("pair/7").exists());
        assert!(d.0.join("pair/8/a.msg").exists());
        assert_eq!(t.get(&manifest_path(SCOPE, 1)).unwrap().unwrap(), b"m");
        t.remove_mailbox("7").unwrap();
        assert!(t.remove_mailbox("../scopes").is_err());
        assert!(d.0.join("scopes").exists());
    }

    #[test]
    fn a_damaged_own_segment_is_replaced_and_another_devices_is_not() {
        let d = dir("replace");
        let t = folder(&d, A);
        let own = segment_path(SCOPE, A, 7);
        assert_eq!(t.create(&own, b"whole bytes"), Put::Created);
        fs::write(d.0.join(&own), b"whole").unwrap();
        t.replace(&own, b"whole bytes").unwrap();
        assert_eq!(fs::read(d.0.join(&own)).unwrap(), b"whole bytes");
        assert_eq!(mode(&d.0.join(&own)), 0o600);
        assert_eq!(
            names(&d.0.join(format!("scopes/{SCOPE}/devices/{A}"))).len(),
            1
        );
        let other = segment_path(SCOPE, B, 7);
        assert!(t.replace(&other, b"x").is_err());
        assert!(t.replace(&manifest_path(SCOPE, 1), b"x").is_err());
        assert!(!d.0.join(&other).exists());
        assert!(!t.keeps());
    }

    #[test]
    fn a_sweep_removes_only_this_devices_old_temporaries() {
        let d = dir("sweep");
        let t = folder(&d, A);
        assert_eq!(t.create(&manifest_path(SCOPE, 1), b"m"), Put::Created);
        assert_eq!(t.create(&segment_path(SCOPE, A, 1), b"s"), Put::Created);
        assert_eq!(t.create(&message_path("7", "a"), b"x"), Put::Created);
        let manifests = d.0.join(format!("scopes/{SCOPE}/manifest"));
        let own = d.0.join(format!("scopes/{SCOPE}/devices/{A}"));
        let ours = format!(".{A}-0123456789abcdef.tmp");
        let theirs = format!(".{B}-0123456789abcdef.tmp");
        for folder in [&manifests, &own, &d.0.join("pair/7")] {
            fs::write(folder.join(&ours), "x").unwrap();
            fs::write(folder.join(&theirs), "x").unwrap();
            fs::write(folder.join(format!(".{A}-0123456789abcdeg.tmp")), "x").unwrap();
            fs::write(folder.join(".other"), "x").unwrap();
        }
        t.sweep(SystemTime::now()).unwrap();
        assert!(manifests.join(&ours).exists());
        let later = SystemTime::now() + Duration::from_secs(2 * 60 * 60);
        t.sweep(later).unwrap();
        for folder in [&manifests, &own, &d.0.join("pair/7")] {
            assert!(!folder.join(&ours).exists(), "{}", folder.display());
            assert!(folder.join(&theirs).exists());
            assert!(folder.join(format!(".{A}-0123456789abcdeg.tmp")).exists());
            assert!(folder.join(".other").exists());
        }
        assert!(manifests.join("1.json").exists());
        assert!(own.join("00000000000000000001.seg").exists());
    }

    #[test]
    fn an_object_over_the_limit_is_refused_and_left_alone() {
        let d = dir("too_big");
        let t = folder(&d, A);
        let path = segment_path(SCOPE, B, 1);
        assert_eq!(t.create(&path, b"x"), Put::Created);
        let file = fs::OpenOptions::new()
            .write(true)
            .open(d.0.join(&path))
            .unwrap();
        file.set_len(OBJECT_MAX + 1).unwrap();
        let why = t.get(&path).unwrap_err();
        assert!(why.contains(&OBJECT_MAX.to_string()), "{why}");
        assert_eq!(fs::metadata(d.0.join(&path)).unwrap().len(), OBJECT_MAX + 1);
        file.set_len(OBJECT_MAX).unwrap();
        assert_eq!(t.get(&path).unwrap().unwrap().len() as u64, OBJECT_MAX);
    }

    #[test]
    fn a_symbolic_link_is_neither_read_nor_written_through() {
        let d = dir("symlink");
        let t = folder(&d, A);
        let outside = d.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret"), "s").unwrap();
        let path = manifest_path(SCOPE, 1);
        fs::create_dir_all(d.0.join(format!("scopes/{SCOPE}/manifest"))).unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), d.0.join(&path)).unwrap();
        assert!(t.get(&path).unwrap_err().contains("symbolic link"));
        assert_eq!(t.create(&path, b"x"), Put::Exists);
        let other = "bcdefghijklmnopqrstuvwxyz3";
        std::os::unix::fs::symlink(&outside, d.0.join(format!("scopes/{other}"))).unwrap();
        assert!(matches!(
            t.create(&manifest_path(other, 1), b"x"),
            Put::Unreachable(_)
        ));
        assert!(!outside.join("manifest").exists());
    }

    #[test]
    fn a_path_outside_the_layout_is_an_error_everywhere() {
        let d = dir("misuse");
        let t = folder(&d, A);
        assert!(t.devices("nope").is_err());
        assert!(t.list_after("nope", A, 0).is_err());
        assert!(t.probe(SCOPE, "nope", 0).is_err());
        assert!(t.highest_manifest("nope").is_err());
        assert!(t.get("scopes/x").is_err());
        assert!(matches!(t.create("scopes/x", b"x"), Put::Unreachable(_)));
    }

    #[test]
    fn a_url_picks_its_transport_by_scheme() {
        let d = dir("open");
        let url = format!("file://{}", d.0.display());
        let t = open(&url, A).unwrap();
        assert!(!t.keeps());
        assert_eq!(t.create(&manifest_path(SCOPE, 1), b"m"), Put::Created);
        assert!(d.0.join(manifest_path(SCOPE, 1)).exists());
        let spaced = d.0.join("My Drive");
        fs::create_dir_all(&spaced).unwrap();
        let t = open(&format!("file://{}", spaced.display()), A).unwrap();
        assert_eq!(t.create(&manifest_path(SCOPE, 1), b"m"), Put::Created);
        let relay = open("https://relay.example", A).err().unwrap();
        assert_eq!(
            relay,
            "https transports are not supported yet; use a file:// folder"
        );
        assert!(open("file://relative/path", A).is_err());
    }
}
