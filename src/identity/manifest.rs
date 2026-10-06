//! Scope manifests: their bytes, validity, sealing, the epoch chain and the versions on disk.
//!
//! ```text
//! <root>/.bilbo/scopes/lock                       held by every writer
//! <root>/.bilbo/scopes/<scope id>/manifest/<n>.json      version n, never rewritten
//! <root>/.bilbo/scopes/<scope id>/manifest/<n>.pending   empty: this device wrote n and no transport holds it yet
//! <root>/.bilbo/scopes/<scope id>/manifest/lost/<n>.json a pending version that lost, kept and never read
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::host::swap;
use crate::identity::keys::{self, BoxSecret, Device, Identity, SignKey};
use crate::shared::{hash, store};

const FORMAT: u32 = 1;
const PREFIX: &[u8] = b"bilbo-manifest-1\n";
/// The `sealed` key of the owner's entry.
pub const OWNER: &str = "owner";
/// An encapsulated key (32), an epoch key (32) and the tag (16).
const SEALED_BYTES: usize = 80;
/// A nonce (24), an epoch key (32) and the tag (16).
const CHAIN_BYTES: usize = 72;
/// A nonce (24) and the tag (16), before the name itself.
const NAME_MIN_BYTES: usize = 41;

/// One listed device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub sign: String,
    #[serde(rename = "box")]
    pub box_key: String,
}

/// An earlier epoch's key, encrypted under the next epoch's key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub epoch: u64,
    pub key: String,
}

/// A manifest as its file holds it. The members are in the order of the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub scope: String,
    pub n: u64,
    pub prev: Option<String>,
    pub owner: String,
    pub owner_box: String,
    pub devices: Vec<Entry>,
    pub transport: String,
    pub epoch: u64,
    pub sealed: BTreeMap<String, String>,
    pub chain: Vec<Link>,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sig: String,
}

impl Manifest {
    /// Whether `id` is a listed device.
    pub fn lists(&self, id: &str) -> bool {
        self.devices.iter().any(|d| d.id == id)
    }
}

/// A device as a manifest lists it, with its keys decoded.
#[derive(Clone, PartialEq)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub sign: [u8; 32],
    pub box_public: [u8; 32],
}

impl Member {
    pub fn of(device: &Device) -> Member {
        let sign = device.sign.public();
        Member {
            id: keys::device_id(&sign),
            name: device.name.clone(),
            sign,
            box_public: device.box_secret.public(),
        }
    }

    pub fn from_entry(entry: &Entry) -> Option<Member> {
        Some(Member {
            id: entry.id.clone(),
            name: entry.name.clone(),
            sign: keys::unhex(&entry.sign)?,
            box_public: keys::unhex(&entry.box_key)?,
        })
    }

    fn entry(&self) -> Entry {
        Entry {
            id: self.id.clone(),
            name: self.name.clone(),
            sign: keys::hex(&self.sign),
            box_key: keys::hex(&self.box_public),
        }
    }
}

/// A version that failed a check, with the reason.
#[derive(Debug, Clone, PartialEq)]
pub struct Invalid {
    pub n: u64,
    pub why: String,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "manifest/{}.json is invalid: {}", self.n, self.why)
    }
}

/// A valid version with the bytes of its file.
pub struct Version {
    pub manifest: Manifest,
    pub bytes: Vec<u8>,
}

/// A scope's versions: the valid prefix, the first failure and the pending markers.
pub struct Scope {
    pub id: String,
    pub versions: Vec<Version>,
    pub invalid: Option<Invalid>,
    pub pending: BTreeSet<u64>,
}

impl Scope {
    /// The latest valid version.
    pub fn latest(&self) -> Option<&Version> {
        self.versions.last()
    }

    /// The owner of version 1, when it is valid.
    pub fn owner(&self) -> Option<[u8; 32]> {
        let first = self.versions.first()?;
        keys::unhex(&first.manifest.owner)
    }
}

/// Lowercase hex of any even length.
fn unhex_vec(text: &str) -> Option<Vec<u8>> {
    let raw = text.as_bytes();
    if !raw.len().is_multiple_of(2) {
        return None;
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    raw.chunks(2)
        .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect()
}

fn body(manifest: &Manifest) -> Vec<u8> {
    let mut unsigned = manifest.clone();
    unsigned.sig.clear();
    serde_json::to_vec(&unsigned).expect("a manifest serializes")
}

fn file_bytes(manifest: &Manifest) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(manifest).expect("a manifest serializes");
    bytes.push(b'\n');
    bytes
}

/// The manifest signed by `key`, with the bytes of its file.
pub fn signed(mut manifest: Manifest, key: &SignKey) -> (Manifest, Vec<u8>) {
    let mut message = PREFIX.to_vec();
    message.extend(body(&manifest));
    manifest.sig = keys::hex(&key.sign(&message));
    let bytes = file_bytes(&manifest);
    (manifest, bytes)
}

/// What the pin of a config URL is: `file://` for a folder, the URL itself otherwise.
pub fn transport_of(url: &str) -> String {
    if url.starts_with("file://") {
        "file://".to_string()
    } else {
        url.to_string()
    }
}

/// Whether a manifest's `transport` agrees with a config URL: whole for `https://` and `http://`, by scheme for
/// `file://`.
pub fn transport_matches(pinned: &str, url: &str) -> bool {
    pinned == transport_of(url)
}

/// Every check that needs no secret, on the file of version `n` of scope `id`, given the valid versions before it.
fn check(id: &str, n: u64, bytes: &[u8], earlier: &[Version]) -> Result<Manifest, String> {
    let (prev, first) = (earlier.last(), earlier.first().map(|v| &v.manifest));
    let m: Manifest =
        serde_json::from_slice(bytes).map_err(|e| format!("it is not a manifest: {e}"))?;
    if file_bytes(&m) != bytes {
        return Err("it is not in canonical form".into());
    }
    if m.format != FORMAT {
        return Err(format!("format {} is not {FORMAT}", m.format));
    }
    let signer: [u8; 32] = keys::unhex(&m.owner).ok_or("owner is not a key")?;
    keys::unhex::<32>(&m.owner_box).ok_or("owner_box is not a key")?;
    let sig: [u8; 64] = keys::unhex(&m.sig).ok_or("sig is not a signature")?;
    let mut message = PREFIX.to_vec();
    message.extend(body(&m));
    if !keys::verify(&signer, &message, &sig) {
        return Err("the signature does not verify with its owner".into());
    }
    if m.scope != id {
        return Err(format!("it names scope {}, not the folder's {id}", m.scope));
    }
    if m.n != n {
        return Err(format!("n is {}, not the file's {n}", m.n));
    }
    if let Some(first) = first {
        if first.owner != m.owner {
            return Err("its owner is not version 1's".into());
        }
        if first.owner_box != m.owner_box {
            return Err("its owner_box is not version 1's".into());
        }
    }
    let want_prev = prev.map(|p| hash::sha256_hex(&p.bytes));
    if m.prev != want_prev {
        return Err("prev is not the SHA-256 of the previous version".into());
    }
    check_devices(&m, earlier)?;
    check_sealed(&m)?;
    check_chain(&m, prev)?;
    if m.name.is_empty() || unhex_vec(&m.name).is_none_or(|b| b.len() < NAME_MIN_BYTES) {
        return Err("name is not a sealed name".into());
    }
    Ok(m)
}

fn check_devices(m: &Manifest, earlier: &[Version]) -> Result<(), String> {
    for entry in &m.devices {
        let sign: [u8; 32] = keys::unhex(&entry.sign).ok_or("a device's sign is not a key")?;
        keys::unhex::<32>(&entry.box_key).ok_or("a device's box is not a key")?;
        if entry.id != keys::device_id(&sign) {
            return Err(format!(
                "device {} is not derived from its sign key",
                entry.id
            ));
        }
        if !keys::valid_name(&entry.name) {
            return Err(format!("device {} has a bad name", entry.id));
        }
    }
    if !m.devices.windows(2).all(|pair| pair[0].id < pair[1].id) {
        return Err("devices are not sorted by id, or an id is listed twice".into());
    }
    for entry in &m.devices {
        let before = earlier
            .iter()
            .flat_map(|v| &v.manifest.devices)
            .find(|d| d.id == entry.id);
        if before.is_some_and(|d| d.box_key != entry.box_key) {
            return Err(format!("device {} has another box than before", entry.id));
        }
    }
    if let Some(prev) = earlier.last().map(|v| &v.manifest)
        && prev.devices.iter().any(|d| !m.lists(&d.id))
        && m.epoch <= prev.epoch
    {
        return Err("a device is dropped without a new epoch".into());
    }
    Ok(())
}

fn check_sealed(m: &Manifest) -> Result<(), String> {
    let wanted: BTreeSet<&str> = m
        .devices
        .iter()
        .map(|d| d.id.as_str())
        .chain([OWNER])
        .collect();
    if m.sealed.keys().map(String::as_str).collect::<BTreeSet<_>>() != wanted {
        return Err("sealed does not hold exactly the listed devices and owner".into());
    }
    for value in m.sealed.values() {
        if unhex_vec(value).is_none_or(|b| b.len() != SEALED_BYTES) {
            return Err("a sealed entry is not an epoch key sealed by HPKE".into());
        }
    }
    Ok(())
}

fn check_chain(m: &Manifest, prev: Option<&Version>) -> Result<(), String> {
    if m.epoch == 0 {
        return Err("epoch counts from 1".into());
    }
    if m.chain.len() as u64 != m.epoch - 1
        || m.chain
            .iter()
            .zip(1u64..)
            .any(|(link, epoch)| link.epoch != epoch)
    {
        return Err(format!(
            "chain does not hold one entry for each epoch from 1 to {}",
            m.epoch - 1
        ));
    }
    if m.chain
        .iter()
        .any(|link| unhex_vec(&link.key).is_none_or(|b| b.len() != CHAIN_BYTES))
    {
        return Err("a chain entry is not an encrypted epoch key".into());
    }
    if let Some(prev) = prev
        && !m.chain.starts_with(&prev.manifest.chain)
    {
        return Err("chain changes an entry of the previous version".into());
    }
    Ok(())
}

/// The secret-free checks on `files`, the bytes of versions 1, 2, … of scope `id`: the valid prefix, and why the
/// next version is not valid, if it is not.
pub fn verify_scope(id: &str, files: &[Vec<u8>]) -> Scope {
    let mut scope = Scope {
        id: id.to_string(),
        versions: Vec::new(),
        invalid: None,
        pending: BTreeSet::new(),
    };
    for (bytes, n) in files.iter().zip(1u64..) {
        match check(id, n, bytes, &scope.versions) {
            Ok(manifest) => scope.versions.push(Version {
                manifest,
                bytes: bytes.clone(),
            }),
            Err(why) => {
                scope.invalid = Some(Invalid { n, why });
                break;
            }
        }
    }
    scope
}

/// The secret-free checks on `bytes` as the version after `scope`'s valid ones, as `verify_scope` runs them on each
/// version: what a relay checks before it takes a new version.
pub fn verify_next(scope: &Scope, bytes: &[u8]) -> Result<Version, String> {
    let n = scope.versions.len() as u64 + 1;
    check(&scope.id, n, bytes, &scope.versions).map(|manifest| Version {
        manifest,
        bytes: bytes.to_vec(),
    })
}

/// Who opens a version: a listed device through its entry, or the owner through the `owner` entry.
pub enum Recipient<'a> {
    Device { id: String, secret: &'a BoxSecret },
    Owner(&'a BoxSecret),
}

impl<'a> Recipient<'a> {
    pub fn device(device: &'a Device) -> Recipient<'a> {
        Recipient::Device {
            id: device.id(),
            secret: &device.box_secret,
        }
    }

    fn label(&self) -> &str {
        match self {
            Recipient::Device { id, .. } => id,
            Recipient::Owner(_) => OWNER,
        }
    }

    fn secret(&self) -> &BoxSecret {
        match self {
            Recipient::Device { secret, .. } | Recipient::Owner(secret) => secret,
        }
    }

    fn listed(&self, m: &Manifest) -> bool {
        m.sealed.contains_key(self.label())
    }
}

type EpochKeys = BTreeMap<u64, Zeroizing<[u8; 32]>>;

/// What a recipient holds of a scope after reading its valid versions.
pub struct Opened {
    /// The latest version, which lists the recipient.
    pub n: u64,
    pub epoch: u64,
    /// The scope's config name, from that version.
    pub name: String,
    /// Every epoch key it holds, from the latest version's entry, the chain and earlier versions.
    pub keys: EpochKeys,
}

/// The epoch keys and the name one version gives `who`, checked against the keys `held` from earlier versions.
fn open_version(
    m: &Manifest,
    who: &Recipient,
    held: &EpochKeys,
) -> Result<(EpochKeys, String), String> {
    if let Recipient::Device { id, secret } = who {
        let listed = m.devices.iter().find(|d| d.id == *id);
        let listed = listed.and_then(|d| keys::unhex::<32>(&d.box_key));
        if listed != Some(secret.public()) {
            return Err("the box key it lists for this device is not this device's".into());
        }
    }
    let sealed = m.sealed.get(who.label()).and_then(|s| unhex_vec(s));
    let sealed = sealed.ok_or("it has no sealed entry for this key")?;
    let key = keys::open_epoch(who.secret(), &m.scope, m.epoch, who.label(), &sealed)?;
    let mut opened: EpochKeys = BTreeMap::new();
    opened.insert(m.epoch, key);
    for link in m.chain.iter().rev() {
        let next = &opened[&(link.epoch + 1)];
        let data = unhex_vec(&link.key).ok_or("a chain entry is not hex")?;
        let plain = keys::decrypt(next, &keys::chain_aad(&m.scope, link.epoch), &data)
            .map_err(|_| format!("the chain entry for epoch {} does not open", link.epoch))?;
        let key: [u8; 32] = plain[..]
            .try_into()
            .map_err(|_| format!("the chain entry for epoch {} is not a key", link.epoch))?;
        opened.insert(link.epoch, Zeroizing::new(key));
    }
    for (epoch, key) in &opened {
        if held.get(epoch).is_some_and(|had| **had != **key) {
            return Err(format!(
                "it holds another key for epoch {epoch} than the one this device has"
            ));
        }
    }
    let name = unhex_vec(&m.name).ok_or("name is not hex")?;
    let aad = keys::name_aad(&m.scope, m.epoch, m.n);
    let name = keys::decrypt(&opened[&m.epoch], &aad, &name)
        .map_err(|_| "the name does not open".to_string())?;
    let name = String::from_utf8(name.to_vec())
        .ok()
        .filter(|n| store::is_topic(n))
        .ok_or("the name is not a scope name")?;
    Ok((opened, name))
}

/// What `who` holds of `scope`, and what it has read of it.
pub struct Reading {
    /// Set when the latest valid version lists the recipient.
    pub opened: Option<Opened>,
    /// The name from the newest version that lists the recipient, even when the latest dropped it.
    pub last_name: Option<String>,
}

/// Reads `scope` as `who`. A version that lists it but fails a check that needs the key is `Err`, and no epoch from it
/// is ever used.
pub fn read(scope: &Scope, who: &Recipient) -> Result<Reading, Invalid> {
    read_versions(&scope.versions, who)
}

fn read_versions(versions: &[Version], who: &Recipient) -> Result<Reading, Invalid> {
    let mut held: EpochKeys = BTreeMap::new();
    let mut latest = None;
    let mut last_name = None;
    for v in versions {
        let m = &v.manifest;
        if !who.listed(m) {
            latest = None;
            continue;
        }
        let (keys, name) = open_version(m, who, &held).map_err(|why| Invalid { n: m.n, why })?;
        for (epoch, key) in keys {
            held.entry(epoch).or_insert(key);
        }
        last_name = Some(name.clone());
        latest = Some((m.n, m.epoch, name));
    }
    let opened = latest.map(|(n, epoch, name)| Opened {
        n,
        epoch,
        name,
        keys: held,
    });
    Ok(Reading { opened, last_name })
}

/// What `who` holds of `scope`: `Ok(None)` when its latest valid version does not list it.
pub fn open(scope: &Scope, who: &Recipient) -> Result<Option<Opened>, Invalid> {
    read(scope, who).map(|r| r.opened)
}

/// The newest epoch a confirmed version introduces that this device holds the key of, the one note data may be
/// encrypted under. It takes what the device opened, so an epoch of a version invalid for it is never named.
pub fn usable_epoch(scope: &Scope, opened: &Opened) -> Option<u64> {
    scope
        .versions
        .iter()
        .filter(|v| v.manifest.n <= opened.n && !scope.pending.contains(&v.manifest.n))
        .map(|v| v.manifest.epoch)
        .filter(|epoch| opened.keys.contains_key(epoch))
        .max()
}

fn manifest_dir(root: &Path, id: &str) -> PathBuf {
    store::scopes_dir(root).join(id).join("manifest")
}

fn version_path(root: &Path, id: &str, n: u64) -> PathBuf {
    manifest_dir(root, id).join(format!("{n}.json"))
}

fn pending_path(root: &Path, id: &str, n: u64) -> PathBuf {
    manifest_dir(root, id).join(format!("{n}.pending"))
}

fn io_message(verb: &str, path: &Path, e: &std::io::Error) -> String {
    format!("cannot {verb} {}: {e}", path.display())
}

/// The ids of the scope folders under `<root>/.bilbo/scopes/`, sorted.
pub fn scope_ids(root: &Path) -> Result<Vec<String>, String> {
    let dir = store::scopes_dir(root);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_message("read", &dir, &e)),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_message("read", &dir, &e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && entry.path().is_dir() {
            ids.push(name);
        }
    }
    ids.sort();
    Ok(ids)
}

/// Reads and checks one scope's versions from disk, with its pending markers. `lost/` and hidden files are not
/// versions.
pub fn read_scope(root: &Path, id: &str) -> Result<Scope, String> {
    let dir = manifest_dir(root, id);
    let mut numbers = BTreeSet::new();
    match fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|e| io_message("read", &dir, &e))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let n = name
                    .strip_suffix(".json")
                    .and_then(|stem| stem.parse::<u64>().ok())
                    .filter(|n| *n > 0 && format!("{n}.json") == name);
                numbers.extend(n);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_message("read", &dir, &e)),
    }
    let mut files = Vec::new();
    for n in 1.. {
        if !numbers.contains(&n) {
            break;
        }
        let path = version_path(root, id, n);
        files.push(fs::read(&path).map_err(|e| io_message("read", &path, &e))?);
    }
    let mut scope = verify_scope(id, &files);
    if scope.invalid.is_none() && numbers.len() > files.len() {
        scope.invalid = Some(Invalid {
            n: files.len() as u64 + 1,
            why: "it is missing, and a later version is there".into(),
        });
    }
    scope.pending = (1..=files.len() as u64)
        .filter(|n| pending_path(root, id, *n).exists())
        .collect();
    Ok(scope)
}

/// A scope as one device sees it.
pub struct Known {
    pub scope: Scope,
    /// Whether the scope is signed by the device's owner.
    pub mine: bool,
    /// What the device holds, when the scope is its owner's and the latest version lists it.
    pub opened: Option<Opened>,
    /// The name from the newest version that lists the device, which the latest version may have dropped.
    pub last_name: Option<String>,
    /// Whether any version lists the device. Only a read the key checks refuse counts as listing it.
    pub ever_listed: bool,
    /// Why the scope's latest version is invalid, for the scope line's problem.
    pub problem: Option<String>,
}

impl Known {
    /// The scope's name, readable only to a listed device.
    pub fn name(&self) -> Option<&str> {
        self.opened.as_ref().map(|o| o.name.as_str())
    }
}

/// Every scope of the store as `who` sees it. `owner` is the device's owner key and `who` its recipient, each
/// `None` on a device without keys. A folder with no version is left out, and one that cannot be read is a scope with
/// a `problem`. The steps below re-check a `Known` under the lock, so surveying before taking it is safe.
pub fn survey(
    root: &Path,
    owner: Option<&[u8; 32]>,
    who: Option<&Recipient>,
) -> Result<Vec<Known>, String> {
    let mut all = Vec::new();
    for id in scope_ids(root)? {
        let scope = match read_scope(root, &id) {
            Ok(scope) => scope,
            Err(problem) => {
                all.push(Known {
                    scope: Scope {
                        id,
                        versions: Vec::new(),
                        invalid: None,
                        pending: BTreeSet::new(),
                    },
                    mine: false,
                    opened: None,
                    last_name: None,
                    ever_listed: false,
                    problem: Some(problem),
                });
                continue;
            }
        };
        if scope.versions.is_empty() && scope.invalid.is_none() {
            continue;
        }
        let mine = owner.is_some() && scope.owner().as_ref() == owner;
        let mut problem = scope.invalid.as_ref().map(Invalid::to_string);
        let (mut opened, mut last_name, mut ever_listed) = (None, None, false);
        if let (true, Some(who)) = (mine, who) {
            match read(&scope, who) {
                Ok(r) => {
                    ever_listed = r.last_name.is_some();
                    (opened, last_name) = (r.opened, r.last_name);
                }
                Err(invalid) => {
                    ever_listed = true;
                    let before = &scope.versions[..(invalid.n - 1) as usize];
                    last_name = read_versions(before, who).ok().and_then(|r| r.last_name);
                    problem = problem.or(Some(invalid.to_string()));
                }
            }
        }
        all.push(Known {
            scope,
            mine,
            opened,
            last_name,
            ever_listed,
            problem,
        });
    }
    Ok(all)
}

/// The devices a new scope lists besides this one, sorted by id: those the latest version of every scope of the owner
/// that this device opened lists, without any device one of those scopes listed once and no longer lists. A manifest
/// this device cannot open vouches for nothing, and a device only some scopes list is not yet the owner's: a thief's
/// scope lists a device `personal` does not, and a revoked device does not come back.
pub fn owner_devices(known: &[Known]) -> Vec<Member> {
    let opened: Vec<&Known> = known
        .iter()
        .filter(|k| k.mine && k.opened.is_some())
        .collect();
    let Some((first, rest)) = opened.split_first() else {
        return Vec::new();
    };
    let latest = |k: &Known| k.scope.latest().map(|v| v.manifest.clone());
    let mut common: Vec<Entry> = latest(first).map(|m| m.devices).unwrap_or_default();
    for k in rest {
        let Some(m) = latest(k) else { continue };
        common.retain(|d| m.lists(&d.id));
    }
    let revoked: BTreeSet<&str> = opened
        .iter()
        .filter_map(|k| k.scope.latest().map(|latest| (k, latest)))
        .flat_map(|(k, latest)| {
            k.scope
                .versions
                .iter()
                .flat_map(|v| &v.manifest.devices)
                .filter(|d| !latest.manifest.lists(&d.id))
                .map(|d| d.id.as_str())
        })
        .collect();
    common
        .iter()
        .filter(|d| !revoked.contains(d.id.as_str()))
        .filter_map(Member::from_entry)
        .collect()
}

/// `<root>/.bilbo/scopes/lock`, held exclusively: every write to a manifest folder goes through a function that
/// takes it.
pub struct Lock {
    root: PathBuf,
    _file: File,
}

impl Lock {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Waits for the lock, creating `scopes/` when it is missing, and removes the hidden temporaries a crash left in the
/// manifest folders: no other writer can be mid-write while this one holds the lock. When the folder is deleted while
/// it waits, the lock it got is on a file nobody else sees, so it opens the new one and takes that.
pub fn lock(root: &Path) -> Result<Lock, String> {
    let dir = store::scopes_dir(root);
    loop {
        fs::create_dir_all(&dir).map_err(|e| io_message("create", &dir, &e))?;
        let path = dir.join("lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| io_message("open", &path, &e))?;
        file.lock().map_err(|e| io_message("lock", &path, &e))?;
        let held = file.metadata().map_err(|e| io_message("stat", &path, &e))?;
        if fs::metadata(&path).is_ok_and(|now| (now.dev(), now.ino()) == (held.dev(), held.ino())) {
            sweep(root)?;
            return Ok(Lock {
                root: root.to_path_buf(),
                _file: file,
            });
        }
    }
}

fn sweep(root: &Path) -> Result<(), String> {
    for id in scope_ids(root)? {
        let dir = manifest_dir(root, &id);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            if hidden && entry.path().is_file() {
                let path = entry.path();
                fs::remove_file(&path).map_err(|e| io_message("remove", &path, &e))?;
            }
        }
    }
    Ok(())
}

/// Writes version `n` whole under a hidden name and moves it into place, refusing to replace a version. A pending
/// version gets its marker first, so a crash leaves at worst a marker with no version.
fn put(lock: &Lock, id: &str, n: u64, bytes: &[u8], pending: bool) -> Result<(), String> {
    let root = lock.root();
    let dir = manifest_dir(root, id);
    fs::create_dir_all(&dir).map_err(|e| io_message("create", &dir, &e))?;
    let path = version_path(root, id, n);
    if path.symlink_metadata().is_ok() {
        return Err(path.display().to_string());
    }
    let marker = pending_path(root, id, n);
    if pending {
        File::create(&marker).map_err(|e| io_message("write", &marker, &e))?;
    } else if let Err(e) = fs::remove_file(&marker)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(io_message("remove", &marker, &e));
    }
    let temporary = stage(&dir, n, bytes)?;
    install(&dir, &temporary, &path)
}

/// Writes `bytes` whole under the hidden name of version `n`.
fn stage(dir: &Path, n: u64, bytes: &[u8]) -> Result<PathBuf, String> {
    let temporary = dir.join(format!(".{n}.new"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| io_message("write", &temporary, &e))?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(io_message("write", &temporary, &e));
    }
    Ok(temporary)
}

/// Moves a staged version into place, refusing to replace one.
fn install(dir: &Path, temporary: &Path, path: &Path) -> Result<(), String> {
    if let Err(message) = swap::rename_new(temporary, path) {
        let _ = fs::remove_file(temporary);
        return Err(message);
    }
    if let Ok(folder) = File::open(dir) {
        let _ = folder.sync_all();
    }
    Ok(())
}

/// A version just written.
#[derive(Debug)]
pub struct Written {
    pub scope: String,
    pub name: String,
    pub n: u64,
    pub epoch: u64,
}

/// Signs `manifest` as the next version after `earlier` (or version 1), checks it as a reader would, and writes it as
/// pending.
fn publish(
    lock: &Lock,
    manifest: Manifest,
    name: &str,
    earlier: &[Version],
    id: &Identity,
) -> Result<Written, String> {
    if manifest.owner != keys::hex(&id.owner.sign.public()) {
        return Err("this device's owner did not sign the scope".into());
    }
    if manifest.owner_box != keys::hex(&id.owner.box_public) {
        return Err("the scope's owner_box is not this device's owner's".into());
    }
    let (manifest, bytes) = signed(manifest, &id.owner.sign);
    check(&manifest.scope, manifest.n, &bytes, earlier)
        .map_err(|why| format!("the next version would be invalid: {why}"))?;
    put(lock, &manifest.scope, manifest.n, &bytes, true)?;
    Ok(Written {
        scope: manifest.scope,
        name: name.to_string(),
        n: manifest.n,
        epoch: manifest.epoch,
    })
}

pub fn seal_all(
    scope: &str,
    epoch: u64,
    key: &[u8; 32],
    members: &[Member],
    owner_box: &[u8; 32],
) -> Result<BTreeMap<String, String>, String> {
    let mut sealed = BTreeMap::new();
    for m in members {
        let value = keys::seal_epoch(&m.box_public, scope, epoch, &m.id, key)?;
        sealed.insert(m.id.clone(), keys::hex(&value));
    }
    let value = keys::seal_epoch(owner_box, scope, epoch, OWNER, key)?;
    sealed.insert(OWNER.to_string(), keys::hex(&value));
    Ok(sealed)
}

/// The sealed name of version `n`, encrypted afresh in every version.
pub fn seal_name(key: &[u8; 32], m: &Manifest, name: &str) -> Result<String, String> {
    let aad = keys::name_aad(&m.scope, m.epoch, m.n);
    let sealed = keys::encrypt(key, &aad, name.as_bytes())?;
    Ok(keys::hex(&sealed))
}

/// The latest version of a scope to build on, which `opened` must be the reading of.
fn base<'a>(scope: &'a Scope, opened: Option<&Opened>) -> Result<&'a Version, String> {
    if let Some(invalid) = &scope.invalid {
        return Err(invalid.to_string());
    }
    let latest = scope.latest().ok_or("the scope has no version")?;
    if opened.is_some_and(|o| o.n != latest.manifest.n) {
        return Err("this device does not hold the latest version".into());
    }
    Ok(latest)
}

fn next(base: &Version) -> Manifest {
    let mut m = base.manifest.clone();
    m.n += 1;
    m.prev = Some(hash::sha256_hex(&base.bytes));
    m.sig.clear();
    m
}

/// Version 1 of a new scope with a fresh id: this device and `others` listed, a new epoch 1 key sealed to each of
/// them and to `owner_box`.
pub fn create(
    lock: &Lock,
    id: &Identity,
    name: &str,
    url: &str,
    others: &[Member],
) -> Result<Written, String> {
    let scope = keys::new_scope_id()?;
    let key = keys::random_secret()?;
    let mut members: BTreeMap<String, Member> =
        others.iter().map(|m| (m.id.clone(), m.clone())).collect();
    let this = Member::of(&id.device);
    members.insert(this.id.clone(), this);
    let members: Vec<Member> = members.into_values().collect();
    let mut manifest = Manifest {
        format: FORMAT,
        scope: scope.clone(),
        n: 1,
        prev: None,
        owner: keys::hex(&id.owner.sign.public()),
        owner_box: keys::hex(&id.owner.box_public),
        devices: members.iter().map(Member::entry).collect(),
        transport: transport_of(url),
        epoch: 1,
        sealed: seal_all(&scope, 1, &key, &members, &id.owner.box_public)?,
        chain: Vec::new(),
        name: String::new(),
        sig: String::new(),
    };
    manifest.name = seal_name(&key, &manifest, name)?;
    publish(lock, manifest, name, &[], id)
}

/// A new version with `url`'s pin and everything else as it was: same epoch, same sealed values, the name encrypted
/// again.
pub fn change_url(
    lock: &Lock,
    scope: &Scope,
    opened: &Opened,
    id: &Identity,
    url: &str,
) -> Result<Written, String> {
    let base = base(scope, Some(opened))?;
    let mut manifest = next(base);
    manifest.transport = transport_of(url);
    let key = opened
        .keys
        .get(&manifest.epoch)
        .ok_or("this device holds no key for the latest epoch")?;
    manifest.name = seal_name(key, &manifest, &opened.name)?;
    publish(lock, manifest, &opened.name, &scope.versions, id)
}

/// A new version with `member` added at the same epoch, its entry sealing `key`, the scope's latest epoch key as the
/// caller opened it (through `owner` for `recover`, through its own entry for pairing). The key is proved against the
/// scope's name first.
pub fn add_device(
    lock: &Lock,
    scope: &Scope,
    key: &[u8; 32],
    member: &Member,
    id: &Identity,
) -> Result<Written, String> {
    let base = base(scope, None)?;
    let mut manifest = next(base);
    if manifest.lists(&member.id) {
        return Err(format!("the scope already lists {}", member.name));
    }
    let sealed_name = unhex_vec(&manifest.name).ok_or("name is not hex")?;
    let old_aad = keys::name_aad(&manifest.scope, manifest.epoch, base.manifest.n);
    let name = keys::decrypt(key, &old_aad, &sealed_name)
        .map_err(|_| "that is not the scope's epoch key".to_string())?;
    let name = String::from_utf8(name.to_vec())
        .ok()
        .filter(|n| store::is_topic(n))
        .ok_or("the scope's name is not a scope name")?;
    manifest.devices.push(member.entry());
    manifest.devices.sort_by(|a, b| a.id.cmp(&b.id));
    let value = keys::seal_epoch(
        &member.box_public,
        &manifest.scope,
        manifest.epoch,
        &member.id,
        key,
    )?;
    manifest.sealed.insert(member.id.clone(), keys::hex(&value));
    manifest.name = seal_name(key, &manifest, &name)?;
    publish(lock, manifest, &name, &scope.versions, id)
}

/// A new version without `target`, under a new random epoch key sealed to the remaining devices and the owner, with
/// the previous epoch's key appended to the chain. Needs the latest epoch key, so only a listed device can write it.
pub fn revoke(
    lock: &Lock,
    scope: &Scope,
    opened: &Opened,
    id: &Identity,
    target: &str,
) -> Result<Written, String> {
    if target == id.device.id() {
        return Err("a device cannot revoke itself".into());
    }
    let base = base(scope, Some(opened))?;
    let mut manifest = next(base);
    if !manifest.lists(target) {
        return Err(format!("the scope does not list {target}"));
    }
    let old = opened
        .keys
        .get(&manifest.epoch)
        .ok_or("this device holds no key for the latest epoch")?;
    let owner_box = id.owner.box_public;
    manifest.devices.retain(|d| d.id != target);
    let members: Vec<Member> = manifest
        .devices
        .iter()
        .map(|d| Member::from_entry(d).ok_or("a device's keys are not keys"))
        .collect::<Result<_, _>>()?;
    let fresh = keys::random_secret()?;
    let link = keys::encrypt(
        &fresh,
        &keys::chain_aad(&manifest.scope, manifest.epoch),
        &old[..],
    )?;
    manifest.chain.push(Link {
        epoch: manifest.epoch,
        key: keys::hex(&link),
    });
    manifest.epoch += 1;
    manifest.sealed = seal_all(
        &manifest.scope,
        manifest.epoch,
        &fresh,
        &members,
        &owner_box,
    )?;
    manifest.name = seal_name(&fresh, &manifest, &opened.name)?;
    publish(lock, manifest, &opened.name, &scope.versions, id)
}

/// Copies version `n`, as a transport holds it, into the store as confirmed: no marker. It must be the next version
/// and pass the secret-free checks against the versions here.
pub fn adopt(lock: &Lock, scope: &str, n: u64, bytes: &[u8]) -> Result<(), String> {
    let known = read_scope(lock.root(), scope)?;
    if let Some(invalid) = &known.invalid {
        return Err(invalid.to_string());
    }
    if n != known.versions.len() as u64 + 1 {
        return Err(format!(
            "version {n} does not follow {} versions",
            known.versions.len()
        ));
    }
    check(scope, n, bytes, &known.versions)?;
    put(lock, scope, n, bytes, false)
}

/// Clears the marker of version `n` when its bytes are the transport's. `false` when this device holds another
/// version `n`, or none.
pub fn confirm(lock: &Lock, scope: &str, n: u64, bytes: &[u8]) -> Result<bool, String> {
    let path = version_path(lock.root(), scope, n);
    match fs::read(&path) {
        Ok(local) if local == bytes => {}
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io_message("read", &path, &e)),
    }
    let marker = pending_path(lock.root(), scope, n);
    match fs::remove_file(&marker) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(io_message("remove", &marker, &e)),
    }
}

/// Moves every version of a scope whose versions are all pending under `manifest/lost/`, clearing their markers, and
/// returns their numbers. It refuses a scope with a confirmed version, which a transport may hold. A step that fails
/// after the first move leaves the versions below it in place, still a valid chain.
pub fn set_aside(lock: &Lock, scope: &str) -> Result<Vec<u64>, String> {
    let root = lock.root();
    let local = read_scope(root, scope)?;
    if let Some(invalid) = &local.invalid {
        return Err(invalid.to_string());
    }
    let last = local.versions.len() as u64;
    if last == 0 || (1..=last).any(|k| !local.pending.contains(&k)) {
        return Err(format!("scope {scope} holds no version or a confirmed one"));
    }
    let graveyard = manifest_dir(root, scope).join("lost");
    fs::create_dir_all(&graveyard).map_err(|e| io_message("create", &graveyard, &e))?;
    let names: Vec<PathBuf> = (1..=last).map(|k| lost_path(&graveyard, k)).collect();
    for k in (1..=last).rev() {
        swap::rename_new(&version_path(root, scope, k), &names[(k - 1) as usize])?;
        let marker = pending_path(root, scope, k);
        fs::remove_file(&marker).map_err(|e| io_message("remove", &marker, &e))?;
    }
    Ok((1..=last).collect())
}

/// What one version changed over the one before it.
enum Change {
    Add(Member),
    Revoke(String),
    Url(String),
}

fn changes(before: &Manifest, after: &Manifest) -> Vec<Change> {
    let mut all = Vec::new();
    for entry in after.devices.iter().filter(|d| !before.lists(&d.id)) {
        all.extend(Member::from_entry(entry).map(Change::Add));
    }
    for entry in before.devices.iter().filter(|d| !after.lists(&d.id)) {
        all.push(Change::Revoke(entry.id.clone()));
    }
    if before.transport != after.transport {
        all.push(Change::Url(after.transport.clone()));
    }
    all
}

/// The result of a pending version losing.
pub struct Lost {
    /// The versions moved to `lost/`.
    pub moved: Vec<u64>,
    /// The pending versions that applied the lost changes again.
    pub written: Vec<Written>,
    /// Changes that were not applied again, with the reason.
    pub skipped: Vec<String>,
    /// A step that failed after the first move, which the moves and writes above stop at. The store stays valid, and a
    /// new `lose` or `adopt` of the same number finishes it.
    pub problem: Option<String>,
}

/// The first free name for lost version `k`: `<k>.json`, then `<k>.2.json`, `<k>.3.json`, … so no loss collides with an
/// earlier one.
fn lost_path(graveyard: &Path, k: u64) -> PathBuf {
    let plain = graveyard.join(format!("{k}.json"));
    if plain.symlink_metadata().is_err() {
        return plain;
    }
    (2..)
        .map(|i| graveyard.join(format!("{k}.{i}.json")))
        .find(|p| p.symlink_metadata().is_err())
        .expect("a free name")
}

/// A transport holds `winner` as version `n`, different from this device's pending one. The pending versions from `n`
/// on move under `manifest/lost/`, `winner` becomes `<n>.json` as confirmed, and each lost change is written again, in
/// order, as a pending version on the winner, as `id`'s device. Everything that can be refused is checked, and every
/// lost name and the winner's file are prepared, before the first move.
pub fn lose(
    lock: &Lock,
    scope: &str,
    n: u64,
    winner: &[u8],
    id: &Identity,
) -> Result<Lost, String> {
    let root = lock.root();
    let local = read_scope(root, scope)?;
    if let Some(invalid) = &local.invalid {
        return Err(invalid.to_string());
    }
    let last = local.versions.len() as u64;
    if n == 0 || n > last {
        return Err(format!("this device holds no version {n}"));
    }
    if (n..=last).any(|k| !local.pending.contains(&k)) {
        return Err(format!("version {n} or a later one is confirmed"));
    }
    let index = (n - 1) as usize;
    if local.versions[index].bytes == winner {
        return Err(format!(
            "the transport holds this device's version {n}; confirm it"
        ));
    }
    check(scope, n, winner, &local.versions[..index])?;

    let mut lost = Vec::new();
    for k in n..=last {
        let after = &local.versions[(k - 1) as usize].manifest;
        let before = (k > 1).then(|| &local.versions[(k - 2) as usize].manifest);
        lost.extend(before.map(|b| changes(b, after)).unwrap_or_default());
    }
    let dir = manifest_dir(root, scope);
    let graveyard = dir.join("lost");
    fs::create_dir_all(&graveyard).map_err(|e| io_message("create", &graveyard, &e))?;
    let names: Vec<PathBuf> = (n..=last).map(|k| lost_path(&graveyard, k)).collect();
    let staged = stage(&dir, n, winner)?;
    let mut report = Lost {
        moved: Vec::new(),
        written: Vec::new(),
        skipped: Vec::new(),
        problem: None,
    };
    for (i, to) in names.iter().enumerate().rev() {
        let k = n + i as u64;
        let step = swap::rename_new(&version_path(root, scope, k), to).and_then(|()| {
            let marker = pending_path(root, scope, k);
            fs::remove_file(&marker).map_err(|e| io_message("remove", &marker, &e))
        });
        if let Err(why) = step {
            let _ = fs::remove_file(&staged);
            if report.moved.is_empty() {
                return Err(why);
            }
            report.moved.reverse();
            report.problem = Some(why);
            return Ok(report);
        }
        report.moved.push(k);
    }
    report.moved.reverse();
    if let Err(why) = install(&dir, &staged, &version_path(root, scope, n)) {
        report.problem = Some(why);
        return Ok(report);
    }

    let me = Recipient::device(&id.device);
    for change in lost {
        let latest = match read_scope(root, scope) {
            Ok(latest) => latest,
            Err(why) => {
                report.problem = Some(why);
                break;
            }
        };
        let opened = match open(&latest, &me) {
            Ok(Some(opened)) => opened,
            Ok(None) => {
                report.skipped.push(
                    "the winning version does not list this device, so its lost changes need the phrase: \
                     run bilbo device recover again"
                        .to_string(),
                );
                break;
            }
            Err(invalid) => {
                report.skipped.push(invalid.to_string());
                break;
            }
        };
        let manifest = &latest.latest().expect("the winner is there").manifest;
        let result = match change {
            Change::Add(member) if manifest.lists(&member.id) => continue,
            Change::Add(member) => {
                add_device(lock, &latest, &opened.keys[&opened.epoch], &member, id)
            }
            Change::Revoke(target) if !manifest.lists(&target) => continue,
            Change::Revoke(target) => revoke(lock, &latest, &opened, id, &target),
            Change::Url(pin) if manifest.transport == pin => continue,
            Change::Url(pin) => change_url(lock, &latest, &opened, id, &pin),
        };
        match result {
            Ok(w) => report.written.push(w),
            Err(why) => report.skipped.push(why),
        }
    }
    Ok(report)
}

/// What one scope's step of `init`, `recover` or `revoke` did.
pub enum Outcome {
    Created(Written),
    Updated(Written),
    Kept,
    /// A syncing scope matched to no manifest while a manifest of the owner has never listed this device.
    Unsealed,
    Failed(String),
}

fn written(result: Result<Written, String>, created: bool) -> Outcome {
    match result {
        Ok(w) if created => Outcome::Created(w),
        Ok(w) => Outcome::Updated(w),
        Err(why) => Outcome::Failed(why),
    }
}

/// Why a `Known` read before the lock no longer matches the disk, if it does not: another writer got in between.
fn stale(lock: &Lock, scope: &Scope) -> Option<String> {
    match read_scope(lock.root(), &scope.id) {
        Ok(now)
            if now.versions.len() == scope.versions.len()
                && now.invalid.is_none() == scope.invalid.is_none()
                && now.latest().map(|v| &v.bytes) == scope.latest().map(|v| &v.bytes) =>
        {
            None
        }
        Ok(_) => Some("the scope changed since it was read; run the command again".into()),
        Err(why) => Some(why),
    }
}

/// What `init` knows about where it runs.
pub struct Context {
    /// Stdin and stderr are terminals and no agent marker is set.
    pub terminal: bool,
    /// Why the scope's transport forbids a version 1 now: it holds a scope this device must join through `recover`,
    /// or one that cannot be attributed, or it cannot be read.
    pub blocked: Option<String>,
}

/// `init`'s step for the scope the config names `name` with `url`. The name matches through the newest version of the
/// owner's scopes that lists this device: a scope whose latest version dropped it is kept and never created again.
/// A scope this device opens is kept, or gets a version with the new pin when `url` differs from its `transport` and a
/// terminal is there. With no match a new scope is created, listing `others` too, unless a manifest of the owner has
/// never listed this device, or is invalid for it before any version it can read: that one may be the scope, so the step is `Unsealed` and mints no id. `off` keeps
/// whatever exists. It never adds this device to a manifest that does not list it. With `ctx.blocked` no version 1 is
/// written.
pub fn init_step(
    lock: &Lock,
    id: &Identity,
    name: &str,
    url: &str,
    known: &[Known],
    others: &[Member],
    ctx: Context,
) -> Outcome {
    if url == "off" {
        return Outcome::Kept;
    }
    let matching: Vec<&Known> = known
        .iter()
        .filter(|k| k.mine && k.last_name.as_deref() == Some(name))
        .collect();
    match matching.as_slice() {
        [] if known.iter().any(|k| {
            k.mine && (!k.ever_listed || (k.problem.is_some() && k.last_name.is_none()))
        }) =>
        {
            Outcome::Unsealed
        }
        [] if ctx.blocked.is_some() => Outcome::Failed(ctx.blocked.unwrap_or_default()),
        [] => written(create(lock, id, name, url, others), true),
        [k] => {
            if let Some(problem) = &k.problem {
                return Outcome::Failed(problem.clone());
            }
            let (Some(opened), Some(latest)) = (k.opened.as_ref(), k.scope.latest()) else {
                return Outcome::Kept;
            };
            if transport_matches(&latest.manifest.transport, url) {
                Outcome::Kept
            } else if !ctx.terminal {
                Outcome::Failed("changing the URL needs a terminal".into())
            } else if let Some(why) = stale(lock, &k.scope) {
                Outcome::Failed(why)
            } else {
                written(change_url(lock, &k.scope, opened, id, url), false)
            }
        }
        _ => Outcome::Failed(format!(
            "more than one manifest of this device is named {name}"
        )),
    }
}

/// `recover`'s step for one scope: when the latest version of this owner's scope does not list this device, a new
/// version adds it, with the epoch key opened through the `owner` entry by `owner_box`.
pub fn recover_step(lock: &Lock, id: &Identity, owner_box: &BoxSecret, known: &Known) -> Outcome {
    if !known.mine {
        return Outcome::Kept;
    }
    if let Some(problem) = &known.problem {
        return Outcome::Failed(problem.clone());
    }
    let this = Member::of(&id.device);
    let Some(latest) = known.scope.latest() else {
        return Outcome::Kept;
    };
    if latest.manifest.lists(&this.id) {
        return Outcome::Kept;
    }
    let opened = match open(&known.scope, &Recipient::Owner(owner_box)) {
        Ok(Some(opened)) => opened,
        Ok(None) => return Outcome::Failed("the owner entry is missing".into()),
        Err(invalid) => return Outcome::Failed(invalid.to_string()),
    };
    let Some(key) = opened.keys.get(&opened.epoch) else {
        return Outcome::Failed("the owner holds no key for the latest epoch".into());
    };
    if let Some(why) = stale(lock, &known.scope) {
        return Outcome::Failed(why);
    }
    written(add_device(lock, &known.scope, key, &this, id), false)
}

/// `revoke`'s step for one scope: a new epoch without `target` when this device opens the scope and the latest
/// version lists `target`, kept when it does not list it.
pub fn revoke_step(lock: &Lock, id: &Identity, known: &Known, target: &str) -> Outcome {
    if target == id.device.id() {
        return Outcome::Failed("a device cannot revoke itself".into());
    }
    if !known.mine && known.scope.owner().is_some() {
        return Outcome::Kept;
    }
    if let Some(problem) = &known.problem {
        return Outcome::Failed(problem.clone());
    }
    let listed = known
        .scope
        .latest()
        .is_some_and(|v| known.mine && v.manifest.lists(target));
    if !listed {
        return Outcome::Kept;
    }
    let Some(opened) = &known.opened else {
        return Outcome::Failed("this device is not listed in the scope".into());
    };
    if let Some(why) = stale(lock, &known.scope) {
        return Outcome::Failed(why);
    }
    written(revoke(lock, &known.scope, opened, id, target), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keys::{Owner, OwnerFile};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("bilbo-manifest-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn ctx(terminal: bool) -> Context {
        Context {
            terminal,
            blocked: None,
        }
    }

    fn owner() -> Owner {
        Owner::derive(&[0; 16])
    }

    fn identity_of(owner: &Owner, name: &str, seed: u8) -> Identity {
        Identity {
            owner: owner.file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    fn ident(name: &str, seed: u8) -> Identity {
        identity_of(&owner(), name, seed)
    }

    fn survey_as(root: &Path, who: &Identity) -> Vec<Known> {
        let owner = who.owner.sign.public();
        survey(root, Some(&owner), Some(&Recipient::device(&who.device))).unwrap()
    }

    fn open_as(scope: &Scope, who: &Identity) -> Result<Option<Opened>, Invalid> {
        open(scope, &Recipient::device(&who.device))
    }

    /// `personal`, pinned to `file://`, at manifest 2 and epoch 1, listing `rhosgobel` and `bywater`.
    struct World {
        root: Scratch,
        rhosgobel: Identity,
        bywater: Identity,
        id: String,
    }

    impl World {
        fn path(&self) -> &Path {
            &self.root.0
        }

        fn scope(&self) -> Scope {
            read_scope(self.path(), &self.id).unwrap()
        }

        fn lock(&self) -> Lock {
            lock(self.path()).unwrap()
        }

        fn confirm_all(&self) {
            let lock = self.lock();
            for v in &self.scope().versions {
                assert!(confirm(&lock, &self.id, v.manifest.n, &v.bytes).unwrap());
            }
        }

        fn revoke_bywater(&self) -> Outcome {
            let known = survey_as(self.path(), &self.rhosgobel);
            revoke_step(
                &self.lock(),
                &self.rhosgobel,
                &known[0],
                &self.bywater.device.id(),
            )
        }
    }

    fn world(name: &str) -> World {
        let root = scratch(name);
        let (rhosgobel, bywater) = (ident("rhosgobel", 1), ident("bywater", 3));
        let lock = lock(&root.0).unwrap();
        let id = create(
            &lock,
            &rhosgobel,
            "personal",
            "file:///Users/a/Sync/bilbo",
            &[],
        )
        .unwrap()
        .scope;
        let known = survey_as(&root.0, &bywater);
        let Outcome::Updated(w) = recover_step(&lock, &bywater, &owner().box_secret, &known[0])
        else {
            panic!("bywater was not added");
        };
        assert_eq!((w.n, w.epoch), (2, 1));
        drop(lock);
        World {
            root,
            rhosgobel,
            bywater,
            id,
        }
    }

    fn usable_of(scope: &Scope, who: &Identity) -> Option<u64> {
        let opened = open_as(scope, who).unwrap().unwrap();
        usable_epoch(scope, &opened)
    }

    fn usable(w: &World) -> Option<u64> {
        usable_of(&w.scope(), &w.rhosgobel)
    }

    fn written(outcome: Outcome) -> Written {
        match outcome {
            Outcome::Created(w) | Outcome::Updated(w) => w,
            Outcome::Kept | Outcome::Unsealed => panic!("no version"),
            Outcome::Failed(why) => panic!("failed: {why}"),
        }
    }

    fn forge(scope: &Scope, n: u64, change: impl FnOnce(&mut Manifest)) -> Vec<u8> {
        let mut m = scope.versions[(n - 1) as usize].manifest.clone();
        change(&mut m);
        signed(m, &owner().sign).1
    }

    fn write_raw(root: &Path, id: &str, n: u64, bytes: &[u8]) {
        fs::create_dir_all(manifest_dir(root, id)).unwrap();
        fs::write(version_path(root, id, n), bytes).unwrap();
    }

    fn bytes_of(scope: &Scope) -> Vec<Vec<u8>> {
        scope.versions.iter().map(|v| v.bytes.clone()).collect()
    }

    fn invalid_n(id: &str, files: &[Vec<u8>]) -> Option<u64> {
        verify_scope(id, files).invalid.map(|i| i.n)
    }

    fn members_of(m: &Manifest) -> Vec<Member> {
        m.devices.iter().filter_map(Member::from_entry).collect()
    }

    fn key_hex(key: &[u8; 32], scope: &str, epoch: u64, plain: &[u8]) -> String {
        keys::hex(&keys::encrypt(key, &keys::chain_aad(scope, epoch), plain).unwrap())
    }

    #[test]
    fn a_first_url_makes_a_scope_folder_of_26_base32_characters() {
        let root = scratch("first_url");
        let me = ident("rhosgobel", 1);
        let lock = lock(&root.0).unwrap();
        let w = create(&lock, &me, "personal", "file:///Users/a/Sync/bilbo", &[]).unwrap();
        let ids = scope_ids(&root.0).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], w.scope);
        assert_eq!(w.scope.len(), 26);
        assert!(
            w.scope
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
        );
        assert_eq!((w.n, w.epoch, w.name.as_str()), (1, 1, "personal"));
    }

    #[test]
    fn off_keeps_the_scope_and_writes_nothing() {
        let w = world("off_keeps");
        let before = bytes_of(&w.scope());
        let known = survey_as(w.path(), &w.rhosgobel);
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            "off",
            &known,
            &[],
            ctx(true),
        );
        assert!(matches!(step, Outcome::Kept));
        assert_eq!(bytes_of(&w.scope()), before);
        let none = scratch("off_none");
        let lock = lock(&none.0).unwrap();
        let step = init_step(&lock, &w.rhosgobel, "personal", "off", &[], &[], ctx(true));
        assert!(matches!(step, Outcome::Kept));
        assert!(scope_ids(&none.0).unwrap().is_empty());
    }

    #[test]
    fn a_renamed_scope_gets_a_new_id() {
        let w = world("renamed");
        let known = survey_as(w.path(), &w.rhosgobel);
        let others = owner_devices(&known);
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "mine",
            "file:///Users/a/Sync/bilbo",
            &known,
            &others,
            ctx(true),
        );
        let created = written(step);
        assert_ne!(created.scope, w.id);
        let names: Vec<_> = survey_as(w.path(), &w.rhosgobel)
            .iter()
            .map(|k| k.name().unwrap().to_string())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"personal".to_string()) && names.contains(&"mine".to_string()));
    }

    #[test]
    fn versions_accumulate_and_the_first_is_never_touched() {
        let w = world("accumulate");
        let first = fs::read(version_path(w.path(), &w.id, 1)).unwrap();
        assert!(matches!(w.revoke_bywater(), Outcome::Updated(_)));
        let mut files: Vec<_> = fs::read_dir(manifest_dir(w.path(), &w.id))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json"))
            .collect();
        files.sort();
        assert_eq!(files, ["1.json", "2.json", "3.json"]);
        assert_eq!(fs::read(version_path(w.path(), &w.id, 1)).unwrap(), first);
        assert!(w.scope().invalid.is_none());
    }

    #[test]
    fn a_version_that_is_already_there_is_left_alone() {
        let w = world("exists");
        let known = survey_as(w.path(), &w.rhosgobel);
        let three = version_path(w.path(), &w.id, 3);
        fs::write(&three, b"mine").unwrap();
        let opened = known[0].opened.as_ref().unwrap();
        let target = w.bywater.device.id();
        let why = revoke(&w.lock(), &known[0].scope, opened, &w.rhosgobel, &target).unwrap_err();
        assert_eq!(why, three.display().to_string());
        assert_eq!(fs::read(&three).unwrap(), b"mine");
        let step = revoke_step(&w.lock(), &w.rhosgobel, &known[0], &target);
        assert!(matches!(step, Outcome::Failed(_)));
    }

    #[test]
    fn a_version_written_here_is_pending_and_one_adopted_is_not() {
        let w = world("pending");
        assert_eq!(w.scope().pending, BTreeSet::from([1, 2]));
        assert!(pending_path(w.path(), &w.id, 1).is_file());
        let other = scratch("pending_copy");
        let lock = lock(&other.0).unwrap();
        for v in &w.scope().versions {
            adopt(&lock, &w.id, v.manifest.n, &v.bytes).unwrap();
        }
        let copy = read_scope(&other.0, &w.id).unwrap();
        assert_eq!(copy.versions.len(), 2);
        assert!(copy.pending.is_empty());
        assert!(!pending_path(&other.0, &w.id, 2).exists());
        let bad = forge(&w.scope(), 2, |m| m.transport = "x".into());
        assert!(adopt(&lock, &w.id, 3, &bad).is_err());
    }

    #[test]
    fn a_pending_epoch_is_not_usable_and_the_next_version_builds_on_it() {
        let w = world("pending_epoch");
        w.confirm_all();
        assert_eq!(usable(&w), Some(1));
        let three = written(w.revoke_bywater());
        assert_eq!((three.n, three.epoch), (3, 2));
        let scope = w.scope();
        assert_eq!(usable_of(&scope, &w.rhosgobel), Some(1));
        let opened = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let carol = Member::of(&ident("carol", 5).device);
        let four = add_device(&w.lock(), &scope, &opened.keys[&2], &carol, &w.rhosgobel).unwrap();
        assert_eq!((four.n, four.epoch), (4, 2));
        let scope = w.scope();
        let v3 = &scope.versions[2];
        assert_eq!(
            scope.versions[3].manifest.prev,
            Some(hash::sha256_hex(&v3.bytes))
        );
        assert_eq!(usable_of(&scope, &w.rhosgobel), Some(1));
        let lock = w.lock();
        assert!(confirm(&lock, &w.id, 3, &v3.bytes).unwrap());
        assert_eq!(usable(&w), Some(2));
    }

    #[test]
    fn nothing_is_usable_until_a_version_is_confirmed() {
        let w = world("nothing_usable");
        assert_eq!(usable(&w), None);
    }

    #[test]
    fn another_devices_version_wins() {
        let w = world("lose");
        w.confirm_all();
        let three = written(w.revoke_bywater());
        assert_eq!(three.n, 3);
        let scope = w.scope();
        let old = scope.versions[2].bytes.clone();
        let winner = forge(&scope, 2, |m| {
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.transport = "https://relay.example.net".into();
            reseal(&w, m);
        });
        let lost = lose(&w.lock(), &w.id, 3, &winner, &w.rhosgobel).unwrap();
        assert_eq!(lost.moved, [3]);
        assert_eq!(lost.written.len(), 1);
        assert!(lost.skipped.is_empty());
        let lost_file = manifest_dir(w.path(), &w.id).join("lost/3.json");
        assert_eq!(fs::read(lost_file).unwrap(), old);
        let scope = w.scope();
        assert!(scope.invalid.is_none());
        assert_eq!(scope.versions[2].bytes, winner);
        assert_eq!(scope.pending, BTreeSet::from([4]));
        let four = &scope.versions[3].manifest;
        assert_eq!(four.epoch, 2);
        assert_eq!(four.transport, "https://relay.example.net");
        assert!(!four.lists(&w.bywater.device.id()));
        let same = lose(&w.lock(), &w.id, 4, &scope.versions[3].bytes, &w.rhosgobel);
        assert!(same.is_err());
    }

    #[test]
    fn a_lost_change_the_winner_already_made_is_not_written_again() {
        let w = world("lose_same");
        w.confirm_all();
        written(w.revoke_bywater());
        let scope = w.scope();
        let mut winner = forge(&scope, 2, |m| m.n = 3);
        winner = forge_from(&winner, |m| {
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.transport = "https://other.example.net".into();
            reseal(&w, m);
        });
        let lost = lose(&w.lock(), &w.id, 3, &winner, &w.rhosgobel).unwrap();
        assert_eq!(lost.moved, [3]);
        assert_eq!(lost.written.len(), 1);
    }

    fn forge_from(bytes: &[u8], change: impl FnOnce(&mut Manifest)) -> Vec<u8> {
        let mut m: Manifest = serde_json::from_slice(bytes).unwrap();
        change(&mut m);
        signed(m, &owner().sign).1
    }

    #[test]
    fn lost_versions_are_kept_and_not_counted() {
        let w = world("lost_kept");
        let graveyard = manifest_dir(w.path(), &w.id).join("lost");
        fs::create_dir_all(&graveyard).unwrap();
        fs::write(graveyard.join("2.json"), b"garbage").unwrap();
        fs::write(graveyard.join("9.json"), b"garbage").unwrap();
        let scope = w.scope();
        assert_eq!(scope.versions.len(), 2);
        assert!(scope.invalid.is_none());
        let _lock = w.lock();
        assert_eq!(fs::read(graveyard.join("9.json")).unwrap(), b"garbage");
    }

    #[test]
    fn the_file_has_its_members_in_order_and_hides_the_name() {
        let w = world("content");
        let scope = w.scope();
        let text = String::from_utf8(scope.versions[0].bytes.clone()).unwrap();
        let order = [
            "\"format\"",
            "\"scope\"",
            "\"n\"",
            "\"prev\"",
            "\"owner\"",
            "\"owner_box\"",
            "\"devices\"",
            "\"transport\"",
            "\"epoch\"",
            "\"sealed\"",
            "\"chain\":[],\"name\":\"",
            ",\"sig\":\"",
        ];
        let at: Vec<usize> = order.iter().map(|m| text.find(m).unwrap()).collect();
        assert!(at.windows(2).all(|p| p[0] < p[1]), "{text}");
        assert!(text.starts_with("{\"format\":1,\"scope\":\""));
        assert!(text.ends_with("\"}\n"));
        assert!(text.contains(&w.id) && text.contains("\"transport\":\"file://\""));
        assert!(text.contains(&keys::hex(&owner().sign.public())));
        assert!(text.contains(&keys::hex(&owner().box_secret.public())));
        assert!(text.contains("\"name\":\"rhosgobel\"") && !text.contains("personal"));
        assert!(text.contains("\"prev\":null") && text.contains("\"chain\":[]"));
    }

    #[test]
    fn an_unknown_member_makes_a_version_invalid() {
        let w = world("unknown_member");
        let text = String::from_utf8(w.scope().versions[0].bytes.clone()).unwrap();
        let bad = text.replacen("\"format\":1,", "\"format\":1,\"extra\":1,", 1);
        assert_eq!(invalid_n(&w.id, &[bad.into_bytes()]), Some(1));
        let hole = text.replacen("\"chain\":[],", "", 1);
        assert_eq!(invalid_n(&w.id, &[hole.into_bytes()]), Some(1));
    }

    #[test]
    fn the_pin_is_whole_for_urls_and_a_scheme_for_folders() {
        assert_eq!(transport_of("file:///Users/a/Dropbox/bilbo"), "file://");
        assert_eq!(
            transport_of("https://relay.example.net"),
            "https://relay.example.net"
        );
        assert!(transport_matches("file://", "file:///home/a/Dropbox/bilbo"));
        assert!(transport_matches(
            "file://",
            "file:///Users/a/Dropbox/bilbo"
        ));
        assert!(transport_matches(
            "https://relay.example.net",
            "https://relay.example.net"
        ));
        assert!(!transport_matches(
            "https://relay.example.net",
            "https://other.example.net"
        ));
        assert!(!transport_matches("file://", "https://relay.example.net"));
        assert!(!transport_matches("https://relay.example.net", "file:///x"));
        assert!(!transport_matches(
            "http://127.0.0.1:8080",
            "http://127.0.0.1:9090"
        ));
    }

    #[test]
    fn a_tampered_or_reformatted_version_is_invalid() {
        let w = world("tampered");
        let files = bytes_of(&w.scope());
        assert_eq!(invalid_n(&w.id, &files), None);
        let mut tampered = files.clone();
        let at = String::from_utf8_lossy(&tampered[1])
            .find("\"devices\"")
            .unwrap()
            + 40;
        tampered[1][at] = if tampered[1][at] == b'a' { b'b' } else { b'a' };
        assert_eq!(invalid_n(&w.id, &tampered), Some(2));
        let value: serde_json::Value = serde_json::from_slice(&files[0]).unwrap();
        let pretty = serde_json::to_vec_pretty(&value).unwrap();
        assert_eq!(invalid_n(&w.id, &[pretty]), Some(1));
        let mut no_newline = files[0].clone();
        no_newline.pop();
        assert_eq!(invalid_n(&w.id, &[no_newline]), Some(1));
    }

    #[test]
    fn a_version_in_another_scopes_folder_is_invalid() {
        let w = world("wrong_folder");
        let files = bytes_of(&w.scope());
        assert_eq!(invalid_n("a".repeat(26).as_str(), &files[..1]), Some(1));
    }

    #[test]
    fn a_broken_prev_is_invalid() {
        let w = world("broken_prev");
        let scope = w.scope();
        let bad = forge(&scope, 2, |m| m.prev = Some(hash::sha256_hex(b"other")));
        assert_eq!(
            invalid_n(&w.id, &[scope.versions[0].bytes.clone(), bad]),
            Some(2)
        );
        let first = forge(&scope, 1, |m| m.prev = Some(hash::sha256_hex(b"other")));
        assert_eq!(invalid_n(&w.id, &[first]), Some(1));
    }

    #[test]
    fn the_next_version_is_checked_as_verify_scope_checks_it() {
        let w = world("verify_next");
        let scope = w.scope();
        let files = bytes_of(&scope);
        for n in 1..files.len() {
            let before = verify_scope(&w.id, &files[..n]);
            let next = verify_next(&before, &files[n]).unwrap();
            assert_eq!(next.bytes, files[n]);
            assert_eq!(next.manifest.n, n as u64 + 1);
        }
        let none = verify_scope(&w.id, &[]);
        assert_eq!(
            verify_next(&none, &files[1]).err().as_deref(),
            Some("n is 2, not the file's 1")
        );
        let first = verify_scope(&w.id, &files[..1]);
        let other = Owner::derive(&[1; 16]);
        let mut m = scope.versions[1].manifest.clone();
        m.owner = keys::hex(&other.sign.public());
        m.owner_box = keys::hex(&other.box_secret.public());
        let (_, bytes) = signed(m, &other.sign);
        assert_eq!(
            verify_next(&first, &bytes).err().as_deref(),
            Some("its owner is not version 1's")
        );
    }

    #[test]
    fn another_owner_cannot_continue_a_scope() {
        let w = world("other_owner");
        let scope = w.scope();
        let other = Owner::derive(&[1; 16]);
        let mut m = scope.versions[1].manifest.clone();
        m.owner = keys::hex(&other.sign.public());
        m.owner_box = keys::hex(&other.box_secret.public());
        let (_, bytes) = signed(m, &other.sign);
        let files = [scope.versions[0].bytes.clone(), bytes];
        let checked = verify_scope(&w.id, &files);
        assert_eq!(checked.invalid.unwrap().why, "its owner is not version 1's");
    }

    #[test]
    fn unsorted_or_repeated_devices_are_invalid() {
        let w = world("unsorted");
        let scope = w.scope();
        let first = scope.versions[0].bytes.clone();
        let unsorted = forge(&scope, 2, |m| m.devices.reverse());
        assert_eq!(invalid_n(&w.id, &[first.clone(), unsorted]), Some(2));
        let twice = forge(&scope, 2, |m| {
            let again = m.devices[0].clone();
            m.devices.insert(0, again);
        });
        assert_eq!(invalid_n(&w.id, &[first.clone(), twice]), Some(2));
        let forged_id = forge(&scope, 2, |m| m.devices[0].id = "a".repeat(26));
        assert_eq!(invalid_n(&w.id, &[first, forged_id]), Some(2));
    }

    #[test]
    fn a_chain_with_a_gap_is_invalid_whoever_reads_it() {
        let w = world("gap");
        let scope = w.scope();
        let first = scope.versions[0].bytes.clone();
        let link = Link {
            epoch: 1,
            key: "00".repeat(CHAIN_BYTES),
        };
        let gap = forge(&scope, 2, |m| {
            m.epoch = 3;
            m.chain = vec![link.clone()];
        });
        assert_eq!(invalid_n(&w.id, &[first.clone(), gap]), Some(2));
        let short = forge(&scope, 2, |m| m.epoch = 2);
        assert_eq!(invalid_n(&w.id, &[first.clone(), short]), Some(2));
        let zero = forge(&scope, 2, |m| m.epoch = 0);
        assert_eq!(invalid_n(&w.id, &[first, zero]), Some(2));
    }

    #[test]
    fn a_later_version_may_not_change_an_earlier_chain_entry() {
        let w = world("chain_prefix");
        w.confirm_all();
        written(w.revoke_bywater());
        let scope = w.scope();
        let files = bytes_of(&scope);
        let v3 = hash::sha256_hex(&files[2]);
        let swapped = forge(&scope, 3, |m| {
            m.n = 4;
            m.prev = Some(v3.clone());
            m.chain[0].key = "11".repeat(CHAIN_BYTES);
        });
        let mut more = files.clone();
        more.push(swapped);
        assert_eq!(invalid_n(&w.id, &more), Some(4));
        let same = forge(&scope, 3, |m| {
            m.n = 4;
            m.prev = Some(v3.clone());
        });
        more.pop();
        more.push(same);
        assert_eq!(invalid_n(&w.id, &more), None);
    }

    #[test]
    fn sealed_must_hold_the_listed_devices_and_the_owner() {
        let w = world("sealed_keys");
        let scope = w.scope();
        let first = scope.versions[0].bytes.clone();
        let extra = forge(&scope, 2, |m| {
            let value = m.sealed[OWNER].clone();
            m.sealed.insert("a".repeat(26), value);
        });
        assert_eq!(invalid_n(&w.id, &[first.clone(), extra]), Some(2));
        let no_owner = forge(&scope, 2, |m| {
            m.sealed.remove(OWNER);
        });
        assert_eq!(invalid_n(&w.id, &[first.clone(), no_owner]), Some(2));
        let short = forge(&scope, 2, |m| {
            m.sealed.insert(OWNER.into(), "00".repeat(SEALED_BYTES - 1));
        });
        assert_eq!(invalid_n(&w.id, &[first, short]), Some(2));
    }

    #[test]
    fn every_listed_device_and_the_owner_open_the_same_epoch_key() {
        let w = world("opens");
        let scope = w.scope();
        let latest = &scope.versions[1].manifest;
        assert_eq!(
            latest
                .sealed
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                OWNER,
                w.rhosgobel.device.id().as_str(),
                w.bywater.device.id().as_str()
            ])
        );
        let a = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let b = open_as(&scope, &w.bywater).unwrap().unwrap();
        let phrase_alone = owner();
        let c = open(&scope, &Recipient::Owner(&phrase_alone.box_secret))
            .unwrap()
            .unwrap();
        assert_eq!((a.n, a.epoch, a.name.as_str()), (2, 1, "personal"));
        assert_eq!(*a.keys[&1], *b.keys[&1]);
        assert_eq!(*a.keys[&1], *c.keys[&1]);
        assert_eq!(c.name, "personal");
    }

    #[test]
    fn an_entry_moved_to_another_recipient_does_not_open() {
        let w = world("moved_entry");
        let scope = w.scope();
        let mine = scope.versions[1].manifest.sealed[&w.rhosgobel.device.id()].clone();
        let moved = forge(&scope, 2, |m| {
            m.sealed.insert(w.bywater.device.id(), mine);
        });
        let files = [scope.versions[0].bytes.clone(), moved];
        let tampered = verify_scope(&w.id, &files);
        assert!(tampered.invalid.is_none());
        assert_eq!(open_as(&tampered, &w.bywater).err().map(|i| i.n), Some(2));
        assert!(open_as(&tampered, &w.rhosgobel).is_ok());
    }

    #[test]
    fn a_device_enrolled_after_revocations_opens_every_epoch_through_the_chain() {
        let root = scratch("late_device");
        let (rhosgobel, bywater, carol, dave) = (
            ident("rhosgobel", 1),
            ident("bywater", 3),
            ident("carol", 5),
            ident("dave", 7),
        );
        let lock = lock(&root.0).unwrap();
        let others = [Member::of(&bywater.device), Member::of(&carol.device)];
        let id = create(&lock, &rhosgobel, "personal", "file://", &others)
            .unwrap()
            .scope;
        for target in [&bywater, &carol] {
            let known = survey_as(&root.0, &rhosgobel);
            written(revoke_step(
                &lock,
                &rhosgobel,
                &known[0],
                &target.device.id(),
            ));
        }
        let known = survey_as(&root.0, &dave);
        let w = written(recover_step(&lock, &dave, &owner().box_secret, &known[0]));
        assert_eq!((w.n, w.epoch), (4, 3));
        let scope = read_scope(&root.0, &id).unwrap();
        assert!(scope.invalid.is_none());
        assert_eq!(scope.versions[3].manifest.chain.len(), 2);
        let (mine, his) = (
            open_as(&scope, &rhosgobel).unwrap().unwrap(),
            open_as(&scope, &dave).unwrap().unwrap(),
        );
        assert_eq!(his.keys.keys().copied().collect::<Vec<_>>(), [1, 2, 3]);
        for epoch in 1..=3 {
            assert_eq!(*mine.keys[&epoch], *his.keys[&epoch]);
        }
        assert_ne!(*his.keys[&1], *his.keys[&2]);
    }

    #[test]
    fn the_first_epoch_has_an_empty_chain_and_a_chain_entry_is_bound_to_its_scope() {
        let w = world("chain_scope");
        w.confirm_all();
        assert!(w.scope().versions[0].manifest.chain.is_empty());
        written(w.revoke_bywater());
        let scope = w.scope();
        let m = &scope.versions[2].manifest;
        assert_eq!(m.chain.len(), 1);
        let opened = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let data = unhex_vec(&m.chain[0].key).unwrap();
        let other = "b".repeat(26);
        assert!(keys::decrypt(&opened.keys[&2], &keys::chain_aad(&other, 1), &data).is_err());
        let plain = keys::decrypt(&opened.keys[&2], &keys::chain_aad(&w.id, 1), &data).unwrap();
        assert_eq!(plain[..], opened.keys[&1][..]);
    }

    #[test]
    fn a_rotation_by_a_device_that_never_held_the_current_key_is_invalid_for_a_member() {
        let w = world("rotation");
        w.confirm_all();
        written(w.revoke_bywater());
        let scope = w.scope();
        let v3 = &scope.versions[2];
        let (k3, wrong) = (
            keys::random_secret().unwrap(),
            keys::random_secret().unwrap(),
        );
        let owner_box = owner().box_secret.public();
        let bytes = forge(&scope, 3, |m| {
            let id = m.scope.clone();
            m.n = 4;
            m.prev = Some(hash::sha256_hex(&v3.bytes));
            m.epoch = 3;
            m.chain.push(Link {
                epoch: 2,
                key: key_hex(&k3, &id, 2, &wrong[..]),
            });
            m.sealed = seal_all(&id, 3, &k3, &members_of(m), &owner_box).unwrap();
            m.name = seal_name(&k3, m, "personal").unwrap();
        });
        write_raw(w.path(), &w.id, 4, &bytes);
        let scope = w.scope();
        assert!(scope.invalid.is_none(), "{:?}", scope.invalid);
        let invalid = open_as(&scope, &w.rhosgobel).err().unwrap();
        assert_eq!(invalid.n, 4);
        let known = survey_as(w.path(), &w.rhosgobel);
        assert!(
            known[0]
                .problem
                .as_ref()
                .unwrap()
                .contains("manifest/4.json")
        );
        assert!(known[0].opened.is_none());
        let step = revoke_step(&w.lock(), &w.rhosgobel, &known[0], &w.bywater.device.id());
        assert!(matches!(step, Outcome::Failed(_)));
    }

    #[test]
    fn a_chain_entry_that_does_not_open_is_invalid_for_a_listed_device() {
        let w = world("chain_wrong_key");
        let scope = w.scope();
        let (k2, wrong) = (
            keys::random_secret().unwrap(),
            keys::random_secret().unwrap(),
        );
        let owner_box = owner().box_secret.public();
        let old = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let bytes = forge(&scope, 2, |m| {
            let id = m.scope.clone();
            m.epoch = 2;
            m.chain = vec![Link {
                epoch: 1,
                key: key_hex(&wrong, &id, 1, &old.keys[&1][..]),
            }];
            m.sealed = seal_all(&id, 2, &k2, &members_of(m), &owner_box).unwrap();
            m.name = seal_name(&k2, m, "personal").unwrap();
        });
        let files = [scope.versions[0].bytes.clone(), bytes];
        let forged = verify_scope(&w.id, &files);
        assert!(forged.invalid.is_none());
        let invalid = open_as(&forged, &w.rhosgobel).err().unwrap();
        assert!(invalid.why.contains("does not open"), "{}", invalid.why);
    }

    #[test]
    fn a_chain_entry_that_opens_to_another_key_than_the_one_held_is_invalid() {
        let w = world("chain_other_key");
        let scope = w.scope();
        let (k2, wrong) = (
            keys::random_secret().unwrap(),
            keys::random_secret().unwrap(),
        );
        let owner_box = owner().box_secret.public();
        let bytes = forge(&scope, 2, |m| {
            let id = m.scope.clone();
            m.epoch = 2;
            m.chain = vec![Link {
                epoch: 1,
                key: key_hex(&k2, &id, 1, &wrong[..]),
            }];
            m.sealed = seal_all(&id, 2, &k2, &members_of(m), &owner_box).unwrap();
            m.name = seal_name(&k2, m, "personal").unwrap();
        });
        let files = [scope.versions[0].bytes.clone(), bytes];
        let forged = verify_scope(&w.id, &files);
        assert!(forged.invalid.is_none());
        let invalid = open_as(&forged, &w.rhosgobel).err().unwrap();
        assert_eq!(invalid.n, 2);
        assert!(invalid.why.contains("epoch 1"), "{}", invalid.why);
        let newcomer = ident("carol", 5);
        assert!(open_as(&forged, &newcomer).unwrap().is_none());
    }

    #[test]
    fn another_key_for_an_epoch_a_member_holds_is_invalid() {
        let w = world("same_epoch");
        let scope = w.scope();
        let fresh = keys::random_secret().unwrap();
        let owner_box = owner().box_secret.public();
        let bytes = forge(&scope, 2, |m| {
            let id = m.scope.clone();
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.sealed = seal_all(&id, 1, &fresh, &members_of(m), &owner_box).unwrap();
            m.name = seal_name(&fresh, m, "personal").unwrap();
        });
        let mut files = bytes_of(&scope);
        files.push(bytes);
        let forged = verify_scope(&w.id, &files);
        assert!(forged.invalid.is_none());
        assert_eq!(open_as(&forged, &w.rhosgobel).err().map(|i| i.n), Some(3));
    }

    #[test]
    fn the_name_opens_under_its_own_epoch_only() {
        let w = world("name_epoch");
        w.confirm_all();
        written(w.revoke_bywater());
        let scope = w.scope();
        let opened = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let m = &scope.versions[2].manifest;
        assert_eq!(m.epoch, 2);
        let name = unhex_vec(&m.name).unwrap();
        let aad = keys::name_aad(&w.id, 2, 3);
        assert_eq!(
            keys::decrypt(&opened.keys[&2], &aad, &name).unwrap()[..],
            b"personal"[..]
        );
        assert!(keys::decrypt(&opened.keys[&1], &aad, &name).is_err());
        let old = unhex_vec(&scope.versions[0].manifest.name).unwrap();
        assert!(keys::decrypt(&opened.keys[&1], &keys::name_aad(&w.id, 1, 1), &old).is_ok());
    }

    #[test]
    fn a_manifest_that_does_not_list_this_device_keeps_its_name_unread() {
        let w = world("unlisted");
        w.confirm_all();
        written(w.revoke_bywater());
        let known = survey_as(w.path(), &w.bywater);
        assert!(known[0].mine && known[0].opened.is_none() && known[0].problem.is_none());
        assert_eq!(known[0].name(), None);
        let nobody = survey(w.path(), Some(&owner().sign.public()), None).unwrap();
        assert_eq!(nobody[0].name(), None);
        assert_eq!(known[0].scope.latest().unwrap().manifest.n, 3);
    }

    #[test]
    fn a_manifest_of_another_owner_is_not_mine() {
        let w = world("foreign");
        let stranger = identity_of(&Owner::derive(&[9; 16]), "mordor", 9);
        let lock = lock(w.path()).unwrap();
        let foreign = create(&lock, &stranger, "theirs", "file://", &[]).unwrap();
        let known = survey_as(w.path(), &w.rhosgobel);
        let theirs = known.iter().find(|k| k.scope.id == foreign.scope).unwrap();
        assert!(!theirs.mine && theirs.opened.is_none() && theirs.problem.is_none());
        assert_eq!(theirs.scope.owner(), Some(stranger.owner.sign.public()));
        let step = recover_step(&lock, &w.rhosgobel, &owner().box_secret, theirs);
        assert!(matches!(step, Outcome::Kept));
        assert!(!owner_devices(&known).iter().any(|m| m.name == "mordor"));
        let broken = crate::shared::store::scopes_dir(w.path())
            .join(&foreign.scope)
            .join("manifest")
            .join("2.json");
        std::fs::write(broken, "{}\n").unwrap();
        let known = survey_as(w.path(), &w.rhosgobel);
        let theirs = known.iter().find(|k| k.scope.id == foreign.scope).unwrap();
        assert!(theirs.problem.is_some());
        let step = revoke_step(&lock, &w.rhosgobel, theirs, &w.bywater.device.id());
        assert!(matches!(step, Outcome::Kept));
    }

    #[test]
    fn a_changed_url_writes_a_version_with_the_same_epoch() {
        let w = world("url_change");
        let known = survey_as(w.path(), &w.rhosgobel);
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            "https://relay.example.net",
            &known,
            &[],
            ctx(true),
        );
        let done = written(step);
        assert_eq!((done.n, done.epoch, done.name.as_str()), (3, 1, "personal"));
        let scope = w.scope();
        let (old, new) = (&scope.versions[1].manifest, &scope.versions[2].manifest);
        assert_eq!(new.transport, "https://relay.example.net");
        assert_eq!((&new.sealed, &new.devices), (&old.sealed, &old.devices));
        assert_ne!(new.name, old.name);
        let known = survey_as(w.path(), &w.rhosgobel);
        let again = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            "https://relay.example.net",
            &known,
            &[],
            ctx(true),
        );
        assert!(matches!(again, Outcome::Kept));
    }

    #[test]
    fn another_folder_path_is_kept_and_a_changed_url_needs_a_terminal() {
        let w = world("url_kept");
        let known = survey_as(w.path(), &w.rhosgobel);
        let moved = "file:///home/a/Sync/bilbo";
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            moved,
            &known,
            &[],
            ctx(false),
        );
        assert!(matches!(step, Outcome::Kept));
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            "https://relay.example.net",
            &known,
            &[],
            ctx(false),
        );
        let Outcome::Failed(why) = step else {
            panic!("wrote without a terminal");
        };
        assert_eq!(why, "changing the URL needs a terminal");
        assert_eq!(w.scope().versions.len(), 2);
    }

    #[test]
    fn a_second_syncing_scope_lists_the_owners_devices_and_seals_to_each() {
        let w = world("second_scope");
        let known = survey_as(w.path(), &w.rhosgobel);
        let others = owner_devices(&known);
        assert_eq!(others.len(), 2);
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "shared",
            "file:///x",
            &known,
            &others,
            ctx(false),
        );
        let created = written(step);
        let scope = read_scope(w.path(), &created.scope).unwrap();
        let m = &scope.versions[0].manifest;
        let ids: BTreeSet<_> = m.devices.iter().map(|d| d.id.clone()).collect();
        assert_eq!(
            ids,
            BTreeSet::from([w.rhosgobel.device.id(), w.bywater.device.id()])
        );
        assert_eq!(m.sealed.len(), 3);
        for who in [&w.rhosgobel, &w.bywater] {
            assert_eq!(open_as(&scope, who).unwrap().unwrap().name, "shared");
        }
    }

    #[test]
    fn recover_adds_this_device_once() {
        let w = world("recover");
        let scope = w.scope();
        let m = &scope.versions[1].manifest;
        assert_eq!(
            (m.n, m.epoch, m.devices.len(), m.sealed.len()),
            (2, 1, 2, 3)
        );
        let known = survey_as(w.path(), &w.bywater);
        let step = recover_step(&w.lock(), &w.bywater, &owner().box_secret, &known[0]);
        assert!(matches!(step, Outcome::Kept));
        assert_eq!(w.scope().versions.len(), 2);
        let c = ident("carol", 5);
        let known = survey_as(w.path(), &c);
        let step = recover_step(&w.lock(), &c, &owner().box_secret, &known[0]);
        assert_eq!(written(step).n, 3);
    }

    #[test]
    fn a_dropped_device_is_not_added_by_init_or_revoke() {
        let w = world("dropped");
        w.confirm_all();
        written(w.revoke_bywater());
        let known = survey_as(w.path(), &w.bywater);
        let step = revoke_step(&w.lock(), &w.bywater, &known[0], &w.rhosgobel.device.id());
        assert!(matches!(step, Outcome::Failed(_)));
        assert_eq!(w.scope().versions.len(), 3);
        let stale = revoke_step(
            &w.lock(),
            &w.rhosgobel,
            &survey_as(w.path(), &w.rhosgobel)[0],
            &w.bywater.device.id(),
        );
        assert!(matches!(stale, Outcome::Kept));
    }

    #[test]
    fn init_never_creates_a_scope_a_manifest_dropped_this_device_from() {
        let w = world("init_dropped");
        w.confirm_all();
        written(w.revoke_bywater());
        let known = survey_as(w.path(), &w.bywater);
        assert_eq!(known[0].last_name.as_deref(), Some("personal"));
        assert!(known[0].ever_listed && known[0].name().is_none());
        let step = init_step(
            &w.lock(),
            &w.bywater,
            "personal",
            "file:///x",
            &known,
            &[],
            ctx(true),
        );
        assert!(matches!(step, Outcome::Kept));
        assert_eq!(scope_ids(w.path()).unwrap(), std::slice::from_ref(&w.id));
        assert_eq!(w.scope().versions.len(), 3);
    }

    #[test]
    fn init_writes_no_version_1_for_an_outsider_and_keeps_every_other_rule() {
        let root = scratch("init_outsider");
        let me = ident("rhosgobel", 1);
        let lock = lock(&root.0).unwrap();
        let blocked = || Context {
            terminal: true,
            blocked: Some("run bilbo device recover on this device".into()),
        };
        let step = init_step(&lock, &me, "personal", "file:///x", &[], &[], blocked());
        let Outcome::Failed(why) = step else {
            panic!("created");
        };
        assert_eq!(why, "run bilbo device recover on this device");
        assert!(scope_ids(&root.0).unwrap().is_empty());
        let step = init_step(&lock, &me, "personal", "off", &[], &[], blocked());
        assert!(matches!(step, Outcome::Kept));
        let step = init_step(&lock, &me, "personal", "file:///x", &[], &[], ctx(true));
        assert!(matches!(step, Outcome::Created(_)));
    }

    #[test]
    fn a_pending_scope_is_set_aside_whole_and_a_confirmed_one_is_refused() {
        let w = world("set_aside");
        let before = w.scope();
        assert_eq!(before.pending, BTreeSet::from([1, 2]));
        let moved = set_aside(&w.lock(), &w.id).unwrap();
        assert_eq!(moved, [1, 2]);
        let dir = manifest_dir(w.path(), &w.id);
        for n in [1u64, 2] {
            assert_eq!(
                fs::read(dir.join(format!("lost/{n}.json"))).unwrap(),
                before.versions[(n - 1) as usize].bytes
            );
            assert!(!dir.join(format!("{n}.json")).exists());
            assert!(!dir.join(format!("{n}.pending")).exists());
        }
        assert!(survey_as(w.path(), &w.rhosgobel).is_empty());
        assert!(set_aside(&w.lock(), &w.id).is_err());
        let damaged = world("set_aside_damaged");
        write_raw(damaged.path(), &damaged.id, 3, b"junk");
        let why = set_aside(&damaged.lock(), &damaged.id).unwrap_err();
        assert!(why.contains("manifest/3.json"), "{why}");
        assert_eq!(damaged.scope().versions.len(), 2);
        let other = world("set_aside_confirmed");
        other.confirm_all();
        let bytes = other.scope().versions[1].bytes.clone();
        assert!(set_aside(&other.lock(), &other.id).is_err());
        assert_eq!(other.scope().versions[1].bytes, bytes);
        assert!(!manifest_dir(other.path(), &other.id).join("lost").exists());
    }

    #[test]
    fn a_second_set_aside_of_the_same_number_keeps_the_first() {
        let w = world("set_aside_twice");
        let first = w.scope();
        set_aside(&w.lock(), &w.id).unwrap();
        for v in &first.versions {
            let n = v.manifest.n;
            write_raw(w.path(), &w.id, n, &v.bytes);
            fs::write(pending_path(w.path(), &w.id, n), "").unwrap();
        }
        set_aside(&w.lock(), &w.id).unwrap();
        let dir = manifest_dir(w.path(), &w.id);
        for (n, v) in (1..).zip(&first.versions) {
            assert_eq!(
                fs::read(dir.join(format!("lost/{n}.json"))).unwrap(),
                v.bytes
            );
            assert_eq!(
                fs::read(dir.join(format!("lost/{n}.2.json"))).unwrap(),
                v.bytes
            );
        }
    }

    #[test]
    fn init_mints_no_id_while_a_manifest_never_listed_this_device() {
        let w = world("init_unsealed");
        let carol = ident("carol", 5);
        let known = survey_as(w.path(), &carol);
        assert!(!known[0].ever_listed && known[0].last_name.is_none());
        for name in ["personal", "shared"] {
            let step = init_step(&w.lock(), &carol, name, "file:///x", &known, &[], ctx(true));
            assert!(matches!(step, Outcome::Unsealed), "{name}");
        }
        assert_eq!(scope_ids(w.path()).unwrap(), std::slice::from_ref(&w.id));
        assert_eq!(w.scope().versions.len(), 2);
    }

    #[test]
    fn revocation_seals_nothing_to_the_revoked_device() {
        let w = world("revoke");
        w.confirm_all();
        let three = written(w.revoke_bywater());
        assert_eq!((three.n, three.epoch), (3, 2));
        let scope = w.scope();
        let m = &scope.versions[2].manifest;
        let gone = w.bywater.device.id();
        assert!(!m.sealed.contains_key(&gone) && !m.lists(&gone));
        assert_eq!(m.sealed.len(), 2);
        assert_eq!(m.chain.len(), 1);
        let (mine, his) = (
            open_as(&scope, &w.rhosgobel).unwrap().unwrap(),
            open_as(&w.scope(), &w.bywater).unwrap(),
        );
        assert!(his.is_none());
        let old = open_as(&scope.versions_up_to(2), &w.bywater)
            .unwrap()
            .unwrap();
        assert_eq!(*old.keys[&1], *mine.keys[&1]);
        for (label, value) in &m.sealed {
            let value = unhex_vec(value).unwrap();
            for as_label in [label.as_str(), gone.as_str(), OWNER] {
                let opened =
                    keys::open_epoch(&w.bywater.device.box_secret, &w.id, 2, as_label, &value);
                assert!(opened.is_err(), "{label} as {as_label}");
            }
        }
        let chain = unhex_vec(&m.chain[0].key).unwrap();
        assert!(keys::decrypt(&old.keys[&1], &keys::chain_aad(&w.id, 1), &chain).is_err());
        let plain = keys::decrypt(&mine.keys[&2], &keys::chain_aad(&w.id, 1), &chain).unwrap();
        assert_eq!(plain[..], old.keys[&1][..]);
        assert_ne!(*mine.keys[&2], *old.keys[&1]);
    }

    impl Scope {
        fn versions_up_to(&self, n: usize) -> Scope {
            verify_scope(&self.id, &bytes_of(self)[..n])
        }
    }

    #[test]
    fn revoke_needs_a_listed_target_and_a_listed_device() {
        let w = world("revoke_args");
        let known = survey_as(w.path(), &w.rhosgobel);
        let opened = known[0].opened.as_ref().unwrap();
        let lock = w.lock();
        let nobody = revoke(
            &lock,
            &known[0].scope,
            opened,
            &w.rhosgobel,
            &"c".repeat(26),
        );
        assert!(nobody.is_err());
        let stranger = identity_of(&Owner::derive(&[2; 16]), "mordor", 9);
        let theirs = revoke(
            &lock,
            &known[0].scope,
            opened,
            &stranger,
            &w.bywater.device.id(),
        );
        assert!(theirs.is_err());
        assert_eq!(w.scope().versions.len(), 2);
    }

    #[test]
    fn a_wrong_epoch_key_adds_nothing() {
        let w = world("wrong_key");
        let carol = Member::of(&ident("carol", 5).device);
        let wrong = keys::random_secret().unwrap();
        let err = add_device(&w.lock(), &w.scope(), &wrong, &carol, &w.rhosgobel).unwrap_err();
        assert!(err.contains("epoch key"));
        let opened = open_as(&w.scope(), &w.rhosgobel).unwrap().unwrap();
        let listed = Member::of(&w.bywater.device);
        assert!(
            add_device(
                &w.lock(),
                &w.scope(),
                &opened.keys[&1],
                &listed,
                &w.rhosgobel
            )
            .is_err()
        );
        assert_eq!(w.scope().versions.len(), 2);
    }

    #[test]
    fn an_invalid_scope_gets_no_next_version() {
        let w = world("invalid_refused");
        let scope = w.scope();
        let bad = forge(&scope, 2, |m| m.devices.reverse());
        let mut files = bytes_of(&scope);
        files.truncate(1);
        files.push(bad);
        fs::write(version_path(w.path(), &w.id, 2), &files[1]).unwrap();
        let known = survey_as(w.path(), &w.rhosgobel);
        assert!(
            known[0]
                .problem
                .as_ref()
                .unwrap()
                .contains("manifest/2.json")
        );
        let step = recover_step(
            &w.lock(),
            &ident("carol", 5),
            &owner().box_secret,
            &known[0],
        );
        assert!(matches!(step, Outcome::Failed(_)));
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            "https://x.example.net",
            &known,
            &[],
            ctx(true),
        );
        assert!(matches!(step, Outcome::Failed(_)));
        assert!(!version_path(w.path(), &w.id, 3).exists());
    }

    #[test]
    fn a_gap_in_the_versions_is_invalid() {
        let w = world("version_gap");
        let two = fs::read(version_path(w.path(), &w.id, 2)).unwrap();
        fs::remove_file(version_path(w.path(), &w.id, 2)).unwrap();
        fs::write(version_path(w.path(), &w.id, 3), two).unwrap();
        let scope = w.scope();
        assert_eq!(scope.versions.len(), 1);
        assert_eq!(scope.invalid.unwrap().n, 2);
    }

    #[test]
    fn a_stale_temporary_is_removed_under_the_lock() {
        let w = world("stale_temp");
        let stale = manifest_dir(w.path(), &w.id).join(".3.new");
        fs::write(&stale, b"half").unwrap();
        let before = bytes_of(&w.scope());
        let _lock = w.lock();
        assert!(!stale.exists());
        assert_eq!(bytes_of(&w.scope()), before);
        assert!(w.scope().invalid.is_none());
    }

    #[test]
    fn two_writers_take_the_lock_in_turn() {
        let w = world("race");
        let (path, id, me) = (w.path(), &w.id, &w.rhosgobel);
        std::thread::scope(|s| {
            for url in ["https://a.example.net", "https://b.example.net"] {
                s.spawn(move || {
                    let lock = lock(path).unwrap();
                    let scope = read_scope(path, id).unwrap();
                    let opened = open_as(&scope, me).unwrap().unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    change_url(&lock, &scope, &opened, me, url).unwrap();
                });
            }
        });
        let scope = w.scope();
        assert!(scope.invalid.is_none());
        assert_eq!(scope.versions.len(), 4);
    }

    #[test]
    fn a_sealed_scope_name_must_be_a_scope_name() {
        let w = world("bad_name");
        let scope = w.scope();
        let key = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let files = [forge(&scope, 1, |m| {
            m.name = seal_name(&key.keys[&1], m, "Not A Name").unwrap()
        })];
        let forged = verify_scope(&w.id, &files);
        assert!(forged.invalid.is_none());
        assert!(open_as(&forged, &w.rhosgobel).is_err());
    }

    /// The name of version `m` encrypted again under the epoch key rhosgobel holds, as an honest writer would.
    fn reseal(w: &World, m: &mut Manifest) {
        let key = open_as(&w.scope(), &w.rhosgobel).unwrap().unwrap().keys[&m.epoch].clone();
        m.name = seal_name(&key, m, "personal").unwrap();
    }

    fn forged_rotation(w: &World, scope: &Scope) -> Vec<u8> {
        let v3 = &scope.versions[2];
        let (k3, wrong) = (
            keys::random_secret().unwrap(),
            keys::random_secret().unwrap(),
        );
        let owner_box = owner().box_secret.public();
        forge(scope, 3, |m| {
            let id = w.id.clone();
            m.n = 4;
            m.prev = Some(hash::sha256_hex(&v3.bytes));
            m.epoch = 3;
            m.chain.push(Link {
                epoch: 2,
                key: key_hex(&k3, &id, 2, &wrong[..]),
            });
            m.sealed = seal_all(&id, 3, &k3, &members_of(m), &owner_box).unwrap();
            m.name = seal_name(&k3, m, "personal").unwrap();
        })
    }

    #[test]
    fn a_changed_box_key_is_invalid_for_every_reader() {
        let w = world("box_swap");
        w.confirm_all();
        written(w.revoke_bywater());
        let scope = w.scope();
        let thief = keys::hex(&w.bywater.device.box_secret.public());
        let riv = w.rhosgobel.device.id();
        let forged = forge(&scope, 3, |m| {
            m.n = 4;
            m.prev = Some(hash::sha256_hex(&scope.versions[2].bytes));
            m.devices.iter_mut().find(|d| d.id == riv).unwrap().box_key = thief;
        });
        let mut files = bytes_of(&scope);
        files.push(forged.clone());
        assert_eq!(invalid_n(&w.id, &files), Some(4));
        write_raw(w.path(), &w.id, 4, &forged);
        let known = survey_as(w.path(), &w.rhosgobel);
        let step = revoke_step(&w.lock(), &w.rhosgobel, &known[0], &w.bywater.device.id());
        assert!(matches!(step, Outcome::Failed(_)));
        assert!(!version_path(w.path(), &w.id, 5).exists());
    }

    #[test]
    fn a_device_dropped_without_a_new_epoch_is_invalid() {
        let w = world("dropped_same_epoch");
        let scope = w.scope();
        let gone = w.bywater.device.id();
        let forged = forge(&scope, 2, |m| {
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.devices.retain(|d| d.id != gone);
            m.sealed.remove(&gone);
        });
        let mut files = bytes_of(&scope);
        files.push(forged);
        assert_eq!(invalid_n(&w.id, &files), Some(3));
    }

    #[test]
    fn a_listed_box_that_is_not_this_devices_does_not_open() {
        let w = world("own_box");
        let scope = w.scope();
        let riv = w.rhosgobel.device.id();
        let other = keys::hex(&w.bywater.device.box_secret.public());
        let bytes = forge(&scope, 1, |m| {
            m.devices.iter_mut().find(|d| d.id == riv).unwrap().box_key = other;
        });
        let forged = verify_scope(&w.id, &[bytes]);
        assert!(forged.invalid.is_none());
        let invalid = open_as(&forged, &w.rhosgobel).err().unwrap();
        assert!(invalid.why.contains("box key"), "{}", invalid.why);
    }

    #[test]
    fn a_changed_owner_box_is_invalid_and_no_writer_publishes_one() {
        let w = world("owner_box");
        let scope = w.scope();
        let thief = keys::hex(&w.bywater.device.box_secret.public());
        let forged = forge(&scope, 2, |m| {
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.owner_box = thief;
        });
        let mut files = bytes_of(&scope);
        files.push(forged);
        let checked = verify_scope(&w.id, &files);
        assert_eq!(
            checked.invalid.unwrap().why,
            "its owner_box is not version 1's"
        );
        let liar = Identity {
            owner: OwnerFile {
                sign: SignKey::from_seed(&owner().sign.seed()),
                box_public: w.bywater.device.box_secret.public(),
            },
            device: Device::from_seeds("rhosgobel", &[1; 32], &[2; 32]),
        };
        let opened = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let err =
            change_url(&w.lock(), &scope, &opened, &liar, "https://x.example.net").unwrap_err();
        assert!(err.contains("owner_box"), "{err}");
        assert_eq!(w.scope().versions.len(), 2);
    }

    #[test]
    fn a_revocation_seals_the_owner_entry_to_the_owners_box() {
        let w = world("owner_entry");
        written(w.revoke_bywater());
        let scope = w.scope();
        let phrase = owner();
        let read = open(&scope, &Recipient::Owner(&phrase.box_secret))
            .unwrap()
            .unwrap();
        assert_eq!(read.epoch, 2);
        let mine = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        assert_eq!(*read.keys[&2], *mine.keys[&2]);
    }

    #[test]
    fn the_intersection_leaves_out_unopened_manifests_and_every_revoked_device() {
        let w = world("intersection");
        let known = survey_as(w.path(), &w.rhosgobel);
        let shared = written(init_step(
            &w.lock(),
            &w.rhosgobel,
            "shared",
            "file:///x",
            &known,
            &owner_devices(&known),
            ctx(true),
        ));
        w.confirm_all();
        let known = survey_as(w.path(), &w.rhosgobel);
        let personal = known.iter().find(|k| k.scope.id == w.id).unwrap();
        let target = w.bywater.device.id();
        written(revoke_step(&w.lock(), &w.rhosgobel, personal, &target));
        let lock = w.lock();
        let evil = create(&lock, &w.bywater, "evil", "file://", &[]).unwrap();
        drop(lock);
        let known = survey_as(w.path(), &w.rhosgobel);
        let theirs = known.iter().find(|k| k.scope.id == evil.scope).unwrap();
        assert!(theirs.mine && theirs.opened.is_none());
        let left = known.iter().find(|k| k.scope.id == shared.scope).unwrap();
        assert!(
            left.scope
                .latest()
                .unwrap()
                .manifest
                .lists(&w.bywater.device.id())
        );
        let ids: Vec<_> = owner_devices(&known).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, [w.rhosgobel.device.id()]);
        let others = owner_devices(&known);
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "third",
            "file:///x",
            &known,
            &others,
            ctx(true),
        );
        assert!(matches!(step, Outcome::Unsealed));
    }

    #[test]
    fn a_second_loss_of_the_same_number_wedges_nothing() {
        let w = world("lost_names");
        w.confirm_all();
        written(w.revoke_bywater());
        let carol = Member::of(&ident("carol", 5).device);
        let scope = w.scope();
        let opened = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        add_device(&w.lock(), &scope, &opened.keys[&2], &carol, &w.rhosgobel).unwrap();
        let scope = w.scope();
        assert_eq!(scope.pending, BTreeSet::from([3, 4]));
        let winner3 = forge(&scope, 2, |m| {
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.transport = "https://relay.example.net".into();
            reseal(&w, m);
        });
        let first = lose(&w.lock(), &w.id, 3, &winner3, &w.rhosgobel).unwrap();
        assert_eq!((first.moved, first.written.len()), (vec![3, 4], 2));
        let scope = w.scope();
        assert_eq!(scope.pending, BTreeSet::from([4, 5]));
        let winner4 = forge(&scope, 3, |m| {
            m.n = 4;
            m.prev = Some(hash::sha256_hex(&scope.versions[2].bytes));
            m.transport = "https://other.example.net".into();
            reseal(&w, m);
        });
        let second = lose(&w.lock(), &w.id, 4, &winner4, &w.rhosgobel).unwrap();
        assert_eq!((second.moved, second.written.len()), (vec![4, 5], 2));
        let scope = w.scope();
        assert!(scope.invalid.is_none());
        assert_eq!(scope.versions.len(), 6);
        assert_eq!(scope.versions[3].bytes, winner4);
        let graveyard = manifest_dir(w.path(), &w.id).join("lost");
        let mut names: Vec<_> = fs::read_dir(&graveyard)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["3.json", "4.2.json", "4.json", "5.json"]);
    }

    #[test]
    fn lose_refuses_before_it_moves_anything() {
        let w = world("lose_guards");
        w.confirm_all();
        let scope = w.scope();
        let other = forge(&scope, 1, |m| {
            m.n = 2;
            m.prev = Some(hash::sha256_hex(&scope.versions[0].bytes));
            m.transport = "https://x.example.net".into();
            reseal(&w, m);
        });
        let confirmed = lose(&w.lock(), &w.id, 2, &other, &w.rhosgobel);
        assert!(confirmed.is_err(), "a confirmed version cannot lose");
        written(w.revoke_bywater());
        let three = fs::read(version_path(w.path(), &w.id, 3)).unwrap();
        for bad in [b"not a manifest\n".to_vec(), three.clone()] {
            assert!(lose(&w.lock(), &w.id, 3, &bad, &w.rhosgobel).is_err());
        }
        assert!(lose(&w.lock(), &w.id, 9, &other, &w.rhosgobel).is_err());
        assert_eq!(fs::read(version_path(w.path(), &w.id, 3)).unwrap(), three);
        assert!(pending_path(w.path(), &w.id, 3).exists());
        assert!(!manifest_dir(w.path(), &w.id).join("lost/3.json").exists());
        assert!(!manifest_dir(w.path(), &w.id).join(".3.new").exists());
    }

    #[test]
    fn a_member_invalid_version_does_not_make_init_fork_the_scope() {
        let w = world("no_fork");
        w.confirm_all();
        written(w.revoke_bywater());
        let forged = forged_rotation(&w, &w.scope());
        write_raw(w.path(), &w.id, 4, &forged);
        let known = survey_as(w.path(), &w.rhosgobel);
        assert!(known[0].problem.is_some() && known[0].opened.is_none());
        assert_eq!(known[0].last_name.as_deref(), Some("personal"));
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            "file:///x",
            &known,
            &[],
            ctx(true),
        );
        assert!(matches!(step, Outcome::Failed(_)));
        assert_eq!(scope_ids(w.path()).unwrap().len(), 1);
    }

    #[test]
    fn the_usable_epoch_is_one_this_device_holds_after_reading() {
        let w = world("usable");
        w.confirm_all();
        written(w.revoke_bywater());
        w.confirm_all();
        let forged = forged_rotation(&w, &w.scope());
        adopt(&w.lock(), &w.id, 4, &forged).unwrap();
        let scope = w.scope();
        assert_eq!(scope.versions[3].manifest.epoch, 3);
        assert!(open_as(&scope, &w.rhosgobel).is_err());
        let before = scope.versions_up_to(3);
        let opened = open_as(&before, &w.rhosgobel).unwrap().unwrap();
        assert_eq!(usable_epoch(&scope, &opened), Some(2));
        let mut keyless = open_as(&before, &w.rhosgobel).unwrap().unwrap();
        keyless.keys.remove(&2);
        assert_eq!(usable_epoch(&scope, &keyless), Some(1));
    }

    #[test]
    fn a_step_on_a_scope_that_changed_since_the_survey_fails_cleanly() {
        let w = world("stale_known");
        let known = survey_as(w.path(), &w.rhosgobel);
        let opened = known[0].opened.as_ref().unwrap();
        change_url(
            &w.lock(),
            &known[0].scope,
            opened,
            &w.rhosgobel,
            "https://a.example.net",
        )
        .unwrap();
        let target = w.bywater.device.id();
        let Outcome::Failed(why) = revoke_step(&w.lock(), &w.rhosgobel, &known[0], &target) else {
            panic!("wrote on a stale scope");
        };
        assert!(why.contains("changed"), "{why}");
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "personal",
            "https://b.example.net",
            &known,
            &[],
            ctx(true),
        );
        assert!(matches!(step, Outcome::Failed(_)));
        assert_eq!(w.scope().versions.len(), 3);
    }

    #[test]
    fn add_device_wants_a_scope_name() {
        let w = world("add_bad_name");
        let scope = w.scope();
        let key = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let files = [forge(&scope, 1, |m| {
            m.name = seal_name(&key.keys[&1], m, "Not A Name").unwrap()
        })];
        let forged = verify_scope(&w.id, &files);
        let carol = Member::of(&ident("carol", 5).device);
        let err = add_device(&w.lock(), &forged, &key.keys[&1], &carol, &w.rhosgobel).unwrap_err();
        assert!(err.contains("scope name"), "{err}");
    }

    #[test]
    fn a_member_with_a_bad_name_is_never_written() {
        let w = world("bad_member");
        let scope = w.scope();
        let opened = open_as(&scope, &w.rhosgobel).unwrap().unwrap();
        let mut carol = Member::of(&ident("carol", 5).device);
        carol.name = "Bad_Name".into();
        let err =
            add_device(&w.lock(), &scope, &opened.keys[&1], &carol, &w.rhosgobel).unwrap_err();
        assert!(err.contains("would be invalid"), "{err}");
        assert_eq!(w.scope().versions.len(), 2);
    }

    #[test]
    fn a_device_cannot_revoke_itself() {
        let w = world("self_revoke");
        let known = survey_as(w.path(), &w.rhosgobel);
        let step = revoke_step(&w.lock(), &w.rhosgobel, &known[0], &w.rhosgobel.device.id());
        assert!(matches!(step, Outcome::Failed(why) if why.contains("itself")));
        let opened = known[0].opened.as_ref().unwrap();
        let direct = revoke(
            &w.lock(),
            &known[0].scope,
            opened,
            &w.rhosgobel,
            &w.rhosgobel.device.id(),
        );
        assert!(direct.is_err());
        assert_eq!(w.scope().versions.len(), 2);
    }

    #[test]
    fn one_unreadable_scope_folder_is_that_scopes_problem() {
        let w = world("unreadable");
        let bad = manifest_dir(w.path(), "unreadablescope").join("1.json");
        fs::create_dir_all(&bad).unwrap();
        let known = survey_as(w.path(), &w.rhosgobel);
        assert_eq!(known.len(), 2);
        let broken = known
            .iter()
            .find(|k| k.scope.id == "unreadablescope")
            .unwrap();
        assert!(broken.problem.is_some() && !broken.mine);
        let fine = known.iter().find(|k| k.scope.id == w.id).unwrap();
        assert_eq!(fine.name(), Some("personal"));
    }

    #[test]
    fn null_prev_wrong_n_and_chain_shapes_are_invalid() {
        let w = world("shapes");
        w.confirm_all();
        written(w.revoke_bywater());
        let scope = w.scope();
        let files = bytes_of(&scope);
        let two = |bytes: Vec<u8>| vec![files[0].clone(), bytes];
        assert_eq!(
            invalid_n(&w.id, &two(forge(&scope, 2, |m| m.prev = None))),
            Some(2)
        );
        assert_eq!(
            invalid_n(&w.id, &two(forge(&scope, 2, |m| m.n = 5))),
            Some(2)
        );
        let v3 = hash::sha256_hex(&files[2]);
        let next = |change: &dyn Fn(&mut Manifest)| {
            let bytes = forge(&scope, 3, |m| {
                m.n = 4;
                m.prev = Some(v3.clone());
                m.epoch = 3;
                m.chain.push(Link {
                    epoch: 2,
                    key: "00".repeat(CHAIN_BYTES),
                });
                change(m);
            });
            let mut all = files.clone();
            all.push(bytes);
            invalid_n(&w.id, &all)
        };
        assert_eq!(next(&|_| {}), None);
        assert_eq!(next(&|m| m.chain[1].epoch = 1), Some(4));
        assert_eq!(
            next(&|m| m.chain[1].key = "00".repeat(CHAIN_BYTES - 1)),
            Some(4)
        );
        assert_eq!(
            next(&|m| m.chain[1].key = "zz".repeat(CHAIN_BYTES)),
            Some(4)
        );
    }

    #[test]
    fn adopt_and_confirm_take_only_what_fits() {
        let w = world("adopt_guards");
        let scope = w.scope();
        let url = |u: &str| {
            forge(&scope, 2, |m| {
                m.n = 3;
                m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
                m.transport = u.into();
            })
        };
        let lock = w.lock();
        assert!(adopt(&lock, &w.id, 4, &url("https://a.example.net")).is_err());
        assert!(adopt(&lock, &w.id, 2, &scope.versions[1].bytes).is_err());
        assert!(!confirm(&lock, &w.id, 2, b"other").unwrap());
        assert!(!confirm(&lock, &w.id, 9, b"other").unwrap());
        assert!(pending_path(w.path(), &w.id, 2).exists());
        fs::write(pending_path(w.path(), &w.id, 3), b"").unwrap();
        adopt(&lock, &w.id, 3, &url("https://a.example.net")).unwrap();
        assert!(!pending_path(w.path(), &w.id, 3).exists());
        assert_eq!(w.scope().pending, BTreeSet::from([1, 2]));
    }

    #[test]
    fn a_stale_reading_cannot_be_built_on() {
        let w = world("stale_opened");
        let old = w.scope();
        let opened = open_as(&old, &w.rhosgobel).unwrap().unwrap();
        change_url(
            &w.lock(),
            &old,
            &opened,
            &w.rhosgobel,
            "https://a.example.net",
        )
        .unwrap();
        let now = w.scope();
        let err = change_url(
            &w.lock(),
            &now,
            &opened,
            &w.rhosgobel,
            "https://b.example.net",
        )
        .unwrap_err();
        assert!(err.contains("latest"), "{err}");
        assert_eq!(w.scope().versions.len(), 3);
    }

    #[test]
    fn a_version_written_without_the_epoch_key_is_invalid_for_every_member() {
        let w = world("name_copied");
        let scope = w.scope();
        let carol = Member::of(&ident("carol", 5).device);
        let readd = forge(&scope, 2, |m| {
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.devices.push(carol.entry());
            m.devices.sort_by(|a, b| a.id.cmp(&b.id));
            m.sealed.insert(carol.id.clone(), "00".repeat(SEALED_BYTES));
        });
        let repoint = forge(&scope, 2, |m| {
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.transport = "https://thief.example.net".into();
        });
        for bytes in [readd, repoint] {
            let mut files = bytes_of(&scope);
            files.push(bytes);
            let forged = verify_scope(&w.id, &files);
            assert!(forged.invalid.is_none());
            for who in [&w.rhosgobel, &w.bywater] {
                let invalid = open_as(&forged, who).err().unwrap();
                assert_eq!(invalid.n, 3);
                assert!(invalid.why.contains("name"), "{}", invalid.why);
            }
        }
    }

    #[test]
    fn every_honest_writer_encrypts_the_name_again() {
        let w = world("name_again");
        let before = w.scope().versions[1].manifest.name.clone();
        let carol = ident("carol", 5);
        let known = survey_as(w.path(), &carol);
        written(recover_step(
            &w.lock(),
            &carol,
            &owner().box_secret,
            &known[0],
        ));
        let after = w.scope().versions[2].manifest.name.clone();
        assert_ne!(before, after);
        assert_eq!(
            open_as(&w.scope(), &carol).unwrap().unwrap().name,
            "personal"
        );
    }

    #[test]
    fn an_id_keeps_its_box_for_life() {
        let w = world("box_for_life");
        w.confirm_all();
        written(w.revoke_bywater());
        let scope = w.scope();
        let gone = w.bywater.device.id();
        let thief = keys::hex(&w.rhosgobel.device.box_secret.public());
        let forged = forge(&scope, 3, |m| {
            m.n = 4;
            m.prev = Some(hash::sha256_hex(&scope.versions[2].bytes));
            let mut entry = Member::of(&w.bywater.device).entry();
            entry.box_key = thief;
            m.devices.push(entry);
            m.devices.sort_by(|a, b| a.id.cmp(&b.id));
            m.sealed.insert(gone.clone(), "00".repeat(SEALED_BYTES));
        });
        let mut files = bytes_of(&scope);
        files.push(forged);
        let checked = verify_scope(&w.id, &files);
        assert!(checked.invalid.unwrap().why.contains("another box"));
        let known = survey_as(w.path(), &w.bywater);
        let step = recover_step(&w.lock(), &w.bywater, &owner().box_secret, &known[0]);
        assert_eq!(written(step).n, 4);
        let scope = w.scope();
        assert!(scope.invalid.is_none());
        let back = scope.versions[3]
            .manifest
            .devices
            .iter()
            .find(|d| d.id == gone);
        assert_eq!(
            back.unwrap().box_key,
            keys::hex(&w.bywater.device.box_secret.public())
        );
        assert!(open_as(&scope, &w.bywater).unwrap().is_some());
    }

    #[test]
    fn a_new_scope_lists_only_what_every_opened_scope_lists() {
        let w = world("intersection");
        let known = survey_as(w.path(), &w.rhosgobel);
        let both = BTreeSet::from([w.rhosgobel.device.id(), w.bywater.device.id()]);
        let ids = |known: &[Known]| -> BTreeSet<String> {
            owner_devices(known).into_iter().map(|m| m.id).collect()
        };
        assert_eq!(ids(&known), both);
        let lock = w.lock();
        let thief = ident("thief", 11);
        create(&lock, &thief, "unopened", "file://", &[]).unwrap();
        let fresh = create(
            &lock,
            &thief,
            "work",
            "file://",
            &[Member::of(&w.rhosgobel.device)],
        )
        .unwrap();
        drop(lock);
        let known = survey_as(w.path(), &w.rhosgobel);
        let theirs = known.iter().find(|k| k.scope.id == fresh.scope).unwrap();
        assert!(theirs.opened.is_some());
        assert_eq!(ids(&known), BTreeSet::from([w.rhosgobel.device.id()]));
        let others = owner_devices(&known);
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "shared",
            "file:///x",
            &known,
            &others,
            ctx(true),
        );
        assert!(matches!(step, Outcome::Created(_) | Outcome::Unsealed));
        if let Outcome::Created(made) = step {
            let scope = read_scope(w.path(), &made.scope).unwrap();
            assert!(open_as(&scope, &thief).unwrap().is_none());
        }
        assert!(owner_devices(&[]).is_empty());
    }

    #[test]
    fn a_manifest_invalid_for_this_device_before_any_readable_version_blocks_minting() {
        let w = world("invalid_v1");
        let scope = w.scope();
        let riv = w.rhosgobel.device.id();
        let bytes = forge(&scope, 1, |m| {
            m.sealed.insert(riv, "00".repeat(SEALED_BYTES));
        });
        let files = [bytes.clone()];
        fs::write(version_path(w.path(), &w.id, 1), &bytes).unwrap();
        fs::remove_file(version_path(w.path(), &w.id, 2)).unwrap();
        assert!(verify_scope(&w.id, &files).invalid.is_none());
        let known = survey_as(w.path(), &w.rhosgobel);
        assert!(known[0].ever_listed && known[0].problem.is_some() && known[0].last_name.is_none());
        let step = init_step(
            &w.lock(),
            &w.rhosgobel,
            "shared",
            "file:///x",
            &known,
            &[],
            ctx(true),
        );
        assert!(matches!(step, Outcome::Unsealed));
        assert_eq!(scope_ids(w.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_lost_recover_is_reported_for_the_user_to_run_again() {
        let w = world("lost_recover");
        w.confirm_all();
        let carol = ident("carol", 5);
        let known = survey_as(w.path(), &carol);
        written(recover_step(
            &w.lock(),
            &carol,
            &owner().box_secret,
            &known[0],
        ));
        let scope = w.scope();
        let winner = forge(&scope, 2, |m| {
            m.n = 3;
            m.prev = Some(hash::sha256_hex(&scope.versions[1].bytes));
            m.transport = "https://relay.example.net".into();
            reseal(&w, m);
        });
        let lost = lose(&w.lock(), &w.id, 3, &winner, &carol).unwrap();
        assert_eq!(lost.moved, [3]);
        assert!(lost.written.is_empty() && lost.problem.is_none());
        assert_eq!(lost.skipped.len(), 1);
        assert!(
            lost.skipped[0].contains("bilbo device recover"),
            "{:?}",
            lost.skipped
        );
        assert!(w.scope().invalid.is_none());
    }
}
