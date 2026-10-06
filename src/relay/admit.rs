//! Who may use a scope on the relay: the access rule, scope admission and the manifest chain, seq contiguity, the
//! quotas, and each scope's state behind its own mutex.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::Flags;
use super::store::Created;
use crate::identity::{keys, manifest};

/// The largest manifest a create may carry: 413 `too-large` above it.
pub const MANIFEST_MAX: u64 = 1 << 20;

const MIB: u64 = 1 << 20;

/// Whether the relay serves a scope it holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Standing {
    Valid,
    /// Its owner was not passed with `--owner`: 403 `not-admitted`.
    NotAdmitted,
    /// It failed the start-up check, for this reason: 403 `invalid`.
    Invalid(String),
}

/// A scope the relay holds.
pub struct Held {
    /// Its valid versions.
    pub chain: manifest::Scope,
    pub standing: Standing,
    /// The bytes of its stored objects.
    pub bytes: u64,
    /// The bytes of bodies being received, booked against `--max-scope-mb`.
    pub reserved: u64,
    /// Each device's highest seq.
    pub highest: BTreeMap<String, u64>,
}

/// What a request on a scope wants to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action<'a> {
    /// Read `manifest/<n>.json` or `manifest/latest`: a listed device, or the owner.
    ReadManifests,
    /// List the device folders or a folder, or read a segment: a listed device only.
    ReadSegments,
    /// Create a segment under `devices/<device>/`: the listed device with that id only.
    WriteSegment { device: &'a str },
}

/// Which side of a scope a request acts as.
#[derive(Debug, Clone, PartialEq)]
pub enum Who {
    /// A device the latest manifest lists, by id.
    Device(String),
    /// The scope's owner.
    Owner,
}

/// Why the relay refuses a request on a scope. The status each maps to is in its line.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// 403 `not-admitted`: an unknown scope, an unlisted key, another device's folder or a scope whose owner was not
    /// passed.
    NotAdmitted,
    /// 403 `invalid`: the start-up check failed the scope.
    Invalid,
    /// 409 `not-next`: a manifest above the latest plus one, or a seq above the device's highest plus one.
    NotNext,
    /// 422 `manifest`: the manifest fails a check, for this reason.
    Manifest(String),
    /// 507 `quota`: a scope or an owner is at its limit.
    Quota,
    /// 413 `too-large`: a body above its cap.
    TooLarge,
}

impl Refusal {
    /// The `error` reason of the response.
    pub fn reason(&self) -> &'static str {
        match self {
            Refusal::NotAdmitted => "not-admitted",
            Refusal::Invalid => "invalid",
            Refusal::NotNext => "not-next",
            Refusal::Manifest(_) => "manifest",
            Refusal::Quota => "quota",
            Refusal::TooLarge => "too-large",
        }
    }
}

/// A create on a scope's object.
pub struct Put<'a> {
    pub scope: &'a str,
    /// The `sign` key the request was verified under.
    pub signer: &'a [u8; 32],
    /// The manifest's `n`, or the segment's seq.
    pub number: u64,
    /// The body's length.
    pub length: u64,
    /// What `Scopes::reserve` booked for this body, `0` when it booked nothing.
    pub reserved: u64,
}

/// The cap on a segment's body.
pub fn segment_max(flags: &Flags) -> u64 {
    flags.max_object_mb * MIB
}

fn scope_max(flags: &Flags) -> u64 {
    flags.max_scope_mb * MIB
}

impl Held {
    /// Which side `key` acts as for `action`, or why it may not. Anything on a scope that is not `Valid` is refused
    /// for its standing, whoever asks.
    pub fn access(&self, key: &[u8; 32], action: Action) -> Result<Who, Refusal> {
        match &self.standing {
            Standing::Valid => {}
            Standing::NotAdmitted => return Err(Refusal::NotAdmitted),
            Standing::Invalid(_) => return Err(Refusal::Invalid),
        }
        let id = keys::device_id(key);
        let listed = self.lists(key);
        match action {
            Action::ReadManifests if listed => Ok(Who::Device(id)),
            Action::ReadManifests if self.chain.owner().as_ref() == Some(key) => Ok(Who::Owner),
            Action::ReadSegments if listed => Ok(Who::Device(id)),
            Action::WriteSegment { device } if listed && id == device => Ok(Who::Device(id)),
            _ => Err(Refusal::NotAdmitted),
        }
    }

    /// Whether the latest version lists `key` as a device.
    fn lists(&self, key: &[u8; 32]) -> bool {
        self.chain.latest().is_some_and(|v| lists(&v.manifest, key))
    }

    /// The valid scope's owner key, when `owners` admits it.
    fn owner_of(&self, owners: &[String]) -> Option<[u8; 32]> {
        self.chain
            .owner()
            .filter(|o| owners.contains(&keys::owner_fingerprint(o)))
    }
}

fn lists(manifest: &manifest::Manifest, key: &[u8; 32]) -> bool {
    let sign = keys::hex(key);
    manifest.devices.iter().any(|d| d.sign == sign)
}

fn locked(held: &Mutex<Held>) -> MutexGuard<'_, Held> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Every scope the relay holds, each behind its own mutex.
///
/// Locking order: the map's mutex first, a scope's mutex second, and never the map while holding a scope. A create's
/// callback runs under the scope's mutex (and, for a new scope, the map's) and must not call back into `Scopes`.
pub struct Scopes {
    /// The admitted owners, as `keys::owner_fingerprint` spells them.
    owners: Vec<String>,
    held: Mutex<BTreeMap<String, Arc<Mutex<Held>>>>,
}

impl Scopes {
    pub fn new(owners: &[String], held: BTreeMap<String, Held>) -> Scopes {
        Scopes {
            owners: owners.to_vec(),
            held: Mutex::new(
                held.into_iter()
                    .map(|(id, h)| (id, Arc::new(Mutex::new(h))))
                    .collect(),
            ),
        }
    }

    fn map(&self) -> MutexGuard<'_, BTreeMap<String, Arc<Mutex<Held>>>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Scope `id`, when the relay holds it.
    pub fn get(&self, id: &str) -> Option<Arc<Mutex<Held>>> {
        self.map().get(id).cloned()
    }

    /// Every scope held, in id order, without keeping the map locked.
    fn all(&self) -> Vec<Arc<Mutex<Held>>> {
        self.map().values().cloned().collect()
    }

    /// Scope `id` and which side `key` acts as for `action`. A scope the relay does not hold is `NotAdmitted`, as one
    /// it holds is for a stranger. The caller locks the scope for what it reads; `Held::access` repeats the check
    /// under that lock.
    pub fn access(
        &self,
        id: &str,
        key: &[u8; 32],
        action: Action,
    ) -> Result<(Arc<Mutex<Held>>, Who), Refusal> {
        let held = self.get(id).ok_or(Refusal::NotAdmitted)?;
        let who = locked(&held).access(key, action)?;
        Ok((held, who))
    }

    /// The valid scopes of the admitted owner `key`, sorted: the answer to `GET /v1/scopes/`. Any other key is
    /// `NotAdmitted`.
    pub fn owner_scopes(&self, key: &[u8; 32]) -> Result<Vec<String>, Refusal> {
        if !self.admits(key) {
            return Err(Refusal::NotAdmitted);
        }
        let mut ids: Vec<String> = self
            .map()
            .iter()
            .filter(|(_, held)| owns(&locked(held), key))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// Whether `key` is the `sign` key of a device that the latest manifest of a valid scope of an admitted owner
    /// lists: who may open a pairing nameplate.
    pub fn enrolled(&self, key: &[u8; 32]) -> bool {
        self.all().iter().any(|held| {
            let held = locked(held);
            held.standing == Standing::Valid
                && held.owner_of(&self.owners).is_some()
                && held.lists(key)
        })
    }

    fn admits(&self, owner: &[u8; 32]) -> bool {
        self.owners.contains(&keys::owner_fingerprint(owner))
    }

    /// Books `length` bytes of a body about to stream against `--max-scope-mb`, under the scope's mutex, and returns
    /// what was booked for `Put::reserved`: `0` when the relay does not hold the scope (a manifest 1 is booked by
    /// nothing). A scope that is not valid is refused for its standing, and one that would pass the cap is `Quota`.
    /// Every reservation ends in exactly one `create_manifest`, `create_segment` or `release`. Call it after
    /// `access` has admitted the key, so a stranger books nothing.
    pub fn reserve(&self, flags: &Flags, id: &str, length: u64) -> Result<u64, Refusal> {
        let Some(held) = self.get(id) else {
            return Ok(0);
        };
        let mut held = locked(&held);
        match &held.standing {
            Standing::Valid => {}
            Standing::NotAdmitted => return Err(Refusal::NotAdmitted),
            Standing::Invalid(_) => return Err(Refusal::Invalid),
        }
        if held.bytes + held.reserved + length > scope_max(flags) {
            return Err(Refusal::Quota);
        }
        held.reserved += length;
        Ok(length)
    }

    /// Gives back what `reserve` booked, for a body that was refused or never finished.
    pub fn release(&self, id: &str, reserved: u64) {
        if let Some(held) = self.get(id) {
            let mut held = locked(&held);
            held.reserved = held.reserved.saturating_sub(reserved);
        }
    }

    /// Admits and creates manifest `put.number` of `put.scope` from `bytes`, whose length is `put.length`.
    ///
    /// Checked under the scope's mutex, in this order: the standing; for an `n` at or below the latest, the key
    /// (listed in that version, in the latest, or the owner) and then the create-only rule (`Same` or `Other` by the
    /// stored bytes, `link` not called); for the next `n`, the key (listed in the latest version, or the owner), then
    /// `manifest::verify_next` (`Manifest(why)`), then the key listed in the new version or the owner; for a larger
    /// `n`, `NotNext`. Manifest 1 of a scope the relay does not hold goes through the same checks, plus `--max-scopes`
    /// valid scopes of the owner, with the map held. `link` creates the object and runs with the lock held: on `New` the version is recorded, so the
    /// next request sees it. Whatever the outcome, `put.reserved` is released.
    pub fn create_manifest(
        &self,
        flags: &Flags,
        put: &Put,
        bytes: &[u8],
        link: &mut dyn FnMut() -> Created,
    ) -> Result<Created, Refusal> {
        match self.get(put.scope) {
            Some(held) => {
                let mut held = locked(&held);
                let result = self.manifest_of_held(flags, &mut held, put, bytes, link);
                held.reserved = held.reserved.saturating_sub(put.reserved);
                result
            }
            None => self.manifest_of_new(flags, put, bytes, link),
        }
    }

    fn manifest_of_held(
        &self,
        flags: &Flags,
        held: &mut Held,
        put: &Put,
        bytes: &[u8],
        link: &mut dyn FnMut() -> Created,
    ) -> Result<Created, Refusal> {
        match &held.standing {
            Standing::Valid => {}
            Standing::NotAdmitted => return Err(Refusal::NotAdmitted),
            Standing::Invalid(_) => return Err(Refusal::Invalid),
        }
        let owner = held.chain.owner();
        let by_owner = owner.as_ref() == Some(put.signer);
        let latest = held.chain.versions.len() as u64;
        if put.number == 0 {
            return Err(Refusal::NotAdmitted);
        }
        if put.number <= latest {
            let stored = &held.chain.versions[put.number as usize - 1];
            if !(by_owner || held.lists(put.signer) || lists(&stored.manifest, put.signer)) {
                return Err(Refusal::NotAdmitted);
            }
            return Ok(if stored.bytes == bytes {
                Created::Same
            } else {
                Created::Other
            });
        }
        if put.number > latest + 1 {
            return if by_owner || held.lists(put.signer) {
                Err(Refusal::NotNext)
            } else {
                Err(Refusal::NotAdmitted)
            };
        }
        if !(by_owner || held.lists(put.signer)) {
            return Err(Refusal::NotAdmitted);
        }
        if put.length > MANIFEST_MAX {
            return Err(Refusal::TooLarge);
        }
        let version = manifest::verify_next(&held.chain, bytes).map_err(Refusal::Manifest)?;
        if !(by_owner || lists(&version.manifest, put.signer)) {
            return Err(Refusal::NotAdmitted);
        }
        if held.bytes + held.reserved.saturating_sub(put.reserved) + put.length > scope_max(flags) {
            return Err(Refusal::Quota);
        }
        let created = link();
        if created == Created::New {
            held.chain.versions.push(version);
            held.bytes += put.length;
        }
        Ok(created)
    }

    fn manifest_of_new(
        &self,
        flags: &Flags,
        put: &Put,
        bytes: &[u8],
        link: &mut dyn FnMut() -> Created,
    ) -> Result<Created, Refusal> {
        if put.number != 1 {
            return Err(Refusal::NotAdmitted);
        }
        if put.length > MANIFEST_MAX {
            return Err(Refusal::TooLarge);
        }
        let empty = manifest::verify_scope(put.scope, &[]);
        let version = manifest::verify_next(&empty, bytes).map_err(Refusal::Manifest)?;
        let owner: [u8; 32] = keys::unhex(&version.manifest.owner)
            .ok_or(Refusal::Manifest("owner is not a key".into()))?;
        if !self.admits(&owner) {
            return Err(Refusal::NotAdmitted);
        }
        if !(put.signer == &owner || lists(&version.manifest, put.signer)) {
            return Err(Refusal::NotAdmitted);
        }
        let mut map = self.map();
        if let Some(held) = map.get(put.scope).cloned() {
            drop(map);
            let mut held = locked(&held);
            return self.manifest_of_held(flags, &mut held, put, bytes, link);
        }
        let count = map
            .values()
            .filter(|held| owns(&locked(held), &owner))
            .count() as u64;
        if count >= flags.max_scopes {
            return Err(Refusal::Quota);
        }
        if put.length > scope_max(flags) {
            return Err(Refusal::Quota);
        }
        let created = link();
        if created == Created::New {
            let mut chain = empty;
            chain.versions.push(version);
            map.insert(
                put.scope.to_string(),
                Arc::new(Mutex::new(Held {
                    chain,
                    standing: Standing::Valid,
                    bytes: put.length,
                    reserved: 0,
                    highest: BTreeMap::new(),
                })),
            );
        }
        Ok(created)
    }

    /// Admits and creates segment `put.number` of `device` in `put.scope`, whose body is `put.length` bytes.
    ///
    /// Checked under the scope's mutex, in this order: the standing, the access rule (`put.signer` is the listed
    /// device `device`), `--max-object-mb`, then the create-only rule: at or below the device's highest seq,
    /// `probe` says whether the stored object is there and has the body's bytes (`Some(true)` is `Same`,
    /// `Some(false)` is `Other`, `None` is `NotNext`, since a segment an operator removed stays missing); at the
    /// next seq, the scope's quota and `link`, which runs with the lock held and records the seq on `New`; above it,
    /// `NotNext`. Whatever the outcome, `put.reserved` is released.
    pub fn create_segment(
        &self,
        flags: &Flags,
        put: &Put,
        device: &str,
        probe: &dyn Fn() -> Result<Option<bool>, String>,
        link: &mut dyn FnMut() -> Created,
    ) -> Result<Created, Refusal> {
        let Some(held) = self.get(put.scope) else {
            return Err(Refusal::NotAdmitted);
        };
        let mut held = locked(&held);
        let result = segment_of_held(flags, &mut held, put, device, probe, link);
        held.reserved = held.reserved.saturating_sub(put.reserved);
        result
    }
}

fn segment_of_held(
    flags: &Flags,
    held: &mut Held,
    put: &Put,
    device: &str,
    probe: &dyn Fn() -> Result<Option<bool>, String>,
    link: &mut dyn FnMut() -> Created,
) -> Result<Created, Refusal> {
    held.access(put.signer, Action::WriteSegment { device })?;
    if put.length > segment_max(flags) {
        return Err(Refusal::TooLarge);
    }
    let highest = held.highest.get(device).copied().unwrap_or(0);
    if put.number == 0 || put.number > highest + 1 {
        return Err(Refusal::NotNext);
    }
    if put.number <= highest {
        return match probe() {
            Ok(Some(true)) => Ok(Created::Same),
            Ok(Some(false)) => Ok(Created::Other),
            Ok(None) => Err(Refusal::NotNext),
            Err(why) => Ok(Created::Failed(why)),
        };
    }
    if held.bytes + held.reserved.saturating_sub(put.reserved) + put.length > scope_max(flags) {
        return Err(Refusal::Quota);
    }
    let created = link();
    if created == Created::New {
        held.highest.insert(device.to_string(), put.number);
        held.bytes += put.length;
    }
    Ok(created)
}

/// Whether `held` is a valid scope signed by `owner`.
fn owns(held: &Held, owner: &[u8; 32]) -> bool {
    held.standing == Standing::Valid && held.chain.owner().as_ref() == Some(owner)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::identity::keys::{Device, Identity, Owner};
    use crate::identity::manifest::{Recipient, Scope};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-admit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn identity(owner: &Owner, name: &str, seed: u8) -> Identity {
        Identity {
            owner: owner.file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    fn print(owner: &Owner) -> String {
        keys::owner_fingerprint(&owner.sign.public())
    }

    fn key(who: &Identity) -> [u8; 32] {
        who.device.sign.public()
    }

    fn flags() -> Flags {
        Flags {
            data: PathBuf::new(),
            owners: Vec::new(),
            listen: "127.0.0.1:0".parse().unwrap(),
            max_scopes: 16,
            max_scope_mb: 1024,
            max_object_mb: 16,
        }
    }

    /// A scope of three versions made as a device would: `rivendell` alone, then `bagend` added, then `bagend`
    /// revoked at epoch 2.
    struct World {
        owner: Owner,
        rivendell: Identity,
        bagend: Identity,
        id: String,
        files: Vec<Vec<u8>>,
        root: Scratch,
    }

    impl World {
        fn scope(&self) -> Scope {
            manifest::read_scope(&self.root.0, &self.id).unwrap()
        }

        fn forged(&self, base: usize, change: impl FnOnce(&mut manifest::Manifest)) -> Vec<u8> {
            let mut m = self.scope().versions[base].manifest.clone();
            change(&mut m);
            manifest::signed(m, &self.owner.sign).1
        }
    }

    fn world(name: &str, seed: u8) -> World {
        let root = scratch(name);
        let owner = Owner::derive(&[seed; 16]);
        let rivendell = identity(&owner, "rivendell", seed + 1);
        let bagend = identity(&owner, "bagend", seed + 3);
        let lock = manifest::lock(&root.0).unwrap();
        let id = manifest::create(&lock, &rivendell, "personal", "file:///x", &[])
            .unwrap()
            .scope;
        let survey = |who: &Identity| {
            let owner_key = who.owner.sign.public();
            manifest::survey(
                &root.0,
                Some(&owner_key),
                Some(&Recipient::device(&who.device)),
            )
            .unwrap()
        };
        let known = survey(&bagend);
        let manifest::Outcome::Updated(_) =
            manifest::recover_step(&lock, &bagend, &owner.box_secret, &known[0])
        else {
            panic!("bagend was not added");
        };
        let known = survey(&rivendell);
        let manifest::Outcome::Updated(w) =
            manifest::revoke_step(&lock, &rivendell, &known[0], &bagend.device.id())
        else {
            panic!("bagend was not revoked");
        };
        assert_eq!((w.n, w.epoch), (3, 2));
        drop(lock);
        let files = manifest::read_scope(&root.0, &id)
            .unwrap()
            .versions
            .iter()
            .map(|v| v.bytes.clone())
            .collect();
        World {
            owner,
            rivendell,
            bagend,
            id,
            files,
            root,
        }
    }

    fn scopes(owners: &[&Owner]) -> Scopes {
        let owners: Vec<String> = owners.iter().map(|o| print(o)).collect();
        Scopes::new(&owners, BTreeMap::new())
    }

    fn put<'a>(scope: &'a str, signer: &'a [u8; 32], number: u64, length: u64) -> Put<'a> {
        Put {
            scope,
            signer,
            number,
            length,
            reserved: 0,
        }
    }

    fn manifest_n(
        scopes: &Scopes,
        w: &World,
        signer: &[u8; 32],
        n: u64,
        bytes: &[u8],
    ) -> Result<Created, Refusal> {
        let length = bytes.len() as u64;
        scopes.create_manifest(&flags(), &put(&w.id, signer, n, length), bytes, &mut || {
            Created::New
        })
    }

    fn admit_to(scopes: &Scopes, w: &World, upto: usize) {
        for (i, bytes) in w.files[..upto].iter().enumerate() {
            let result = manifest_n(scopes, w, &key(&w.rivendell), i as u64 + 1, bytes);
            assert_eq!(result, Ok(Created::New), "manifest {}", i + 1);
        }
    }

    fn segment(
        scopes: &Scopes,
        flags: &Flags,
        w: &World,
        who: &Identity,
        seq: u64,
        probe: Option<bool>,
    ) -> Result<Created, Refusal> {
        scopes.create_segment(
            flags,
            &put(&w.id, &key(who), seq, 10),
            &w.rivendell.device.id(),
            &|| Ok(probe),
            &mut || Created::New,
        )
    }

    fn status(scopes: &Scopes, id: &str) -> (u64, u64) {
        let held = scopes.get(id).unwrap();
        let held = locked(&held);
        (held.bytes, held.reserved)
    }

    #[test]
    fn a_new_scope_enters_through_its_first_manifest() {
        let w = world("new_scope", 10);
        let scopes = scopes(&[&w.owner]);
        assert!(scopes.get(&w.id).is_none());
        admit_to(&scopes, &w, 1);
        let held = scopes.get(&w.id).unwrap();
        let held = locked(&held);
        assert_eq!(held.standing, Standing::Valid);
        assert_eq!(held.chain.versions.len(), 1);
        assert_eq!(held.bytes, w.files[0].len() as u64);
        drop(held);
        let access = |who: &Identity| {
            scopes
                .access(&w.id, &key(who), Action::ReadSegments)
                .map(|(_, who)| who)
        };
        assert_eq!(
            access(&w.rivendell),
            Ok(Who::Device(w.rivendell.device.id()))
        );
    }

    #[test]
    fn an_owner_not_passed_keeps_nothing() {
        let w = world("not_passed", 10);
        let other = Owner::derive(&[99; 16]);
        let scopes = scopes(&[&other]);
        let result = manifest_n(&scopes, &w, &key(&w.rivendell), 1, &w.files[0]);
        assert_eq!(result, Err(Refusal::NotAdmitted));
        assert!(scopes.get(&w.id).is_none());
    }

    #[test]
    fn a_forged_owner_signature_is_a_manifest_failure() {
        let w = world("forged", 10);
        let scopes = scopes(&[&w.owner]);
        let mut m = w.scope().versions[0].manifest.clone();
        m.sig = "0".repeat(128);
        let mut bytes = serde_json::to_vec(&m).unwrap();
        bytes.push(b'\n');
        let result = manifest_n(&scopes, &w, &key(&w.rivendell), 1, &bytes);
        assert!(matches!(result, Err(Refusal::Manifest(_))), "{result:?}");
        assert!(scopes.get(&w.id).is_none());
    }

    #[test]
    fn the_uploader_must_be_listed_or_the_owner() {
        let w = world("uploader", 10);
        let scopes = scopes(&[&w.owner]);
        let stranger = Device::from_seeds("x", &[77; 32], &[78; 32]).sign.public();
        let result = manifest_n(&scopes, &w, &stranger, 1, &w.files[0]);
        assert_eq!(result, Err(Refusal::NotAdmitted));
        assert!(scopes.get(&w.id).is_none());
        let by_owner = manifest_n(&scopes, &w, &w.owner.sign.public(), 1, &w.files[0]);
        assert_eq!(by_owner, Ok(Created::New));
    }

    #[test]
    fn a_manifest_above_the_unheld_first_is_not_admitted() {
        let w = world("unheld_n", 10);
        let scopes = scopes(&[&w.owner]);
        let result = manifest_n(&scopes, &w, &w.owner.sign.public(), 2, &w.files[1]);
        assert_eq!(result, Err(Refusal::NotAdmitted));
    }

    #[test]
    fn the_chain_grows_by_one() {
        let w = world("chain", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 1);
        let rivendell = key(&w.rivendell);
        assert_eq!(
            manifest_n(&scopes, &w, &rivendell, 3, &w.files[2]),
            Err(Refusal::NotNext)
        );
        assert_eq!(
            manifest_n(&scopes, &w, &rivendell, 2, &w.files[1]),
            Ok(Created::New)
        );
        let held = scopes.get(&w.id).unwrap();
        assert_eq!(locked(&held).chain.versions.len(), 2);
    }

    #[test]
    fn a_stored_manifest_follows_the_create_only_rule() {
        let w = world("create_only", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let rivendell = key(&w.rivendell);
        assert_eq!(
            manifest_n(&scopes, &w, &rivendell, 3, &w.files[2]),
            Ok(Created::Same)
        );
        let other = w.forged(1, |m| m.name.push_str("00"));
        assert_eq!(
            manifest_n(&scopes, &w, &rivendell, 3, &other),
            Ok(Created::Other)
        );
        assert_eq!(
            manifest_n(&scopes, &w, &rivendell, 2, &other),
            Ok(Created::Other)
        );
        let stranger = Device::from_seeds("x", &[77; 32], &[78; 32]).sign.public();
        assert_eq!(
            manifest_n(&scopes, &w, &stranger, 3, &other),
            Err(Refusal::NotAdmitted)
        );
    }

    #[test]
    fn the_link_runs_under_the_lock_and_a_failure_records_nothing() {
        let w = world("link_fails", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 1);
        let rivendell = key(&w.rivendell);
        let before = status(&scopes, &w.id);
        for failure in [
            Created::Full("disk".into()),
            Created::Failed("io".into()),
            Created::Other,
        ] {
            let result = scopes.create_manifest(
                &flags(),
                &put(&w.id, &rivendell, 2, w.files[1].len() as u64),
                &w.files[1],
                &mut || {
                    let held = scopes.get(&w.id).unwrap();
                    assert!(held.try_lock().is_err(), "the scope is locked");
                    failure.clone_for_test()
                },
            );
            assert_eq!(result, Ok(failure));
        }
        assert_eq!(status(&scopes, &w.id), before);
        let held = scopes.get(&w.id).unwrap();
        assert_eq!(locked(&held).chain.versions.len(), 1);
    }

    #[test]
    fn a_changed_owner_is_a_manifest_failure() {
        let w = world("owner_changed", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 1);
        let rival = Owner::derive(&[55; 16]);
        let mut m = w.scope().versions[0].manifest.clone();
        m.n = 2;
        m.prev = Some(crate::shared::hash::sha256_hex(&w.files[0]));
        m.owner = keys::hex(&rival.sign.public());
        let bytes = manifest::signed(m, &rival.sign).1;
        let result = manifest_n(&scopes, &w, &key(&w.rivendell), 2, &bytes);
        assert!(matches!(result, Err(Refusal::Manifest(_))), "{result:?}");
    }

    #[test]
    fn a_wrong_prev_is_a_manifest_failure() {
        let w = world("prev", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 1);
        let bytes = w.forged(1, |m| m.prev = Some("0".repeat(64)));
        let result = manifest_n(&scopes, &w, &key(&w.rivendell), 2, &bytes);
        assert!(matches!(result, Err(Refusal::Manifest(why)) if why.contains("prev")));
    }

    #[test]
    fn a_chain_with_an_epoch_missing_or_rewritten_is_a_manifest_failure() {
        let w = world("chain_rules", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let rivendell = key(&w.rivendell);
        let fourth = |change: &dyn Fn(&mut manifest::Manifest)| {
            w.forged(2, |m| {
                m.n = 4;
                m.prev = Some(crate::shared::hash::sha256_hex(&w.files[2]));
                change(m);
            })
        };
        let missing = fourth(&|m| m.epoch = 3);
        let result = manifest_n(&scopes, &w, &rivendell, 4, &missing);
        assert!(matches!(&result, Err(Refusal::Manifest(why)) if why.contains("chain")));
        let rewritten = fourth(&|m| m.chain[0].key = "00".repeat(72));
        let result = manifest_n(&scopes, &w, &rivendell, 4, &rewritten);
        assert!(matches!(&result, Err(Refusal::Manifest(why)) if why.contains("chain")));
        assert_eq!(
            manifest_n(&scopes, &w, &rivendell, 4, &fourth(&|_| ())),
            Ok(Created::New)
        );
    }

    #[test]
    fn the_owner_recovers_a_scope_by_listing_a_device() {
        let w = world("recovery", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 2);
        let bagend = key(&w.bagend);
        assert_eq!(
            manifest_n(&scopes, &w, &bagend, 3, &w.files[2]),
            Err(Refusal::NotAdmitted),
            "version 3 lists only rivendell"
        );
        assert_eq!(
            manifest_n(&scopes, &w, &w.owner.sign.public(), 3, &w.files[2]),
            Ok(Created::New)
        );
    }

    #[test]
    fn a_revoked_device_is_not_admitted() {
        let w = world("revoked", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        for action in [Action::ReadManifests, Action::ReadSegments] {
            let result = scopes.access(&w.id, &key(&w.bagend), action).map(|r| r.1);
            assert_eq!(result, Err(Refusal::NotAdmitted), "{action:?}");
        }
    }

    #[test]
    fn a_device_writes_only_its_own_folder() {
        let w = world("own_folder", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let rivendell = key(&w.rivendell);
        let own = w.rivendell.device.id();
        let other = w.bagend.device.id();
        let access = |device: &str| {
            scopes
                .access(&w.id, &rivendell, Action::WriteSegment { device })
                .map(|r| r.1)
        };
        assert_eq!(access(&own), Ok(Who::Device(own.clone())));
        assert_eq!(access(&other), Err(Refusal::NotAdmitted));
        let result = scopes.create_segment(
            &flags(),
            &put(&w.id, &rivendell, 1, 10),
            &other,
            &|| Ok(None),
            &mut || panic!("linked another device's folder"),
        );
        assert_eq!(result, Err(Refusal::NotAdmitted));
    }

    #[test]
    fn the_owner_reads_manifests_and_nothing_else() {
        let w = world("owner_reads", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let owner = w.owner.sign.public();
        let who = |action| scopes.access(&w.id, &owner, action).map(|r| r.1);
        assert_eq!(who(Action::ReadManifests), Ok(Who::Owner));
        assert_eq!(who(Action::ReadSegments), Err(Refusal::NotAdmitted));
        let own = w.rivendell.device.id();
        assert_eq!(
            who(Action::WriteSegment { device: &own }),
            Err(Refusal::NotAdmitted)
        );
    }

    #[test]
    fn a_key_that_is_nobody_learns_nothing() {
        let w = world("nobody", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let nobody = Device::from_seeds("n", &[90; 32], &[91; 32]).sign.public();
        let held = scopes
            .access(&w.id, &nobody, Action::ReadManifests)
            .map(|r| r.1);
        let unknown = scopes
            .access("nonexistentnonexistentnone", &nobody, Action::ReadManifests)
            .map(|r| r.1);
        assert_eq!(held, Err(Refusal::NotAdmitted));
        assert_eq!(unknown, held);
    }

    #[test]
    fn a_scope_that_is_not_valid_is_refused_for_its_standing() {
        let w = world("standing", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let rivendell = key(&w.rivendell);
        let held = scopes.get(&w.id).unwrap();
        for (standing, refusal) in [
            (Standing::NotAdmitted, Refusal::NotAdmitted),
            (Standing::Invalid("why".into()), Refusal::Invalid),
        ] {
            locked(&held).standing = standing;
            let access = scopes.access(&w.id, &rivendell, Action::ReadManifests);
            assert_eq!(access.map(|r| r.1), Err(refusal.clone()));
            assert_eq!(scopes.reserve(&flags(), &w.id, 1), Err(refusal.clone()));
            let created = manifest_n(&scopes, &w, &rivendell, 2, &w.files[1]);
            assert_eq!(created, Err(refusal.clone()));
            let result = segment(&scopes, &flags(), &w, &w.rivendell, 1, None);
            assert_eq!(result, Err(refusal));
            assert!(!scopes.enrolled(&rivendell));
        }
    }

    #[test]
    fn a_segment_is_the_next_seq() {
        let w = world("seqs", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let flags = flags();
        let seg = |seq, probe| segment(&scopes, &flags, &w, &w.rivendell, seq, probe);
        assert_eq!(seg(2, None), Err(Refusal::NotNext));
        assert_eq!(seg(0, None), Err(Refusal::NotNext));
        assert_eq!(seg(1, None), Ok(Created::New));
        assert_eq!(seg(2, None), Ok(Created::New));
        assert_eq!(seg(4, None), Err(Refusal::NotNext));
        assert_eq!(seg(3, None), Ok(Created::New));
        assert_eq!(seg(3, Some(true)), Ok(Created::Same));
        assert_eq!(seg(2, Some(false)), Ok(Created::Other));
        assert_eq!(
            seg(1, None),
            Err(Refusal::NotNext),
            "an operator removed it"
        );
        let held = scopes.get(&w.id).unwrap();
        let held = locked(&held);
        assert_eq!(held.highest[&w.rivendell.device.id()], 3);
    }

    #[test]
    fn a_failed_segment_link_records_nothing() {
        let w = world("seg_fail", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let before = status(&scopes, &w.id);
        let rivendell = key(&w.rivendell);
        let result = scopes.create_segment(
            &flags(),
            &put(&w.id, &rivendell, 1, 10),
            &w.rivendell.device.id(),
            &|| Ok(None),
            &mut || Created::Full("disk".into()),
        );
        assert_eq!(result, Ok(Created::Full("disk".into())));
        assert_eq!(status(&scopes, &w.id), before);
        let seg = segment(&scopes, &flags(), &w, &w.rivendell, 1, None);
        assert_eq!(seg, Ok(Created::New));
    }

    #[test]
    fn a_segment_above_the_object_cap_is_too_large() {
        let w = world("object_cap", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let flags = Flags {
            max_object_mb: 1,
            ..flags()
        };
        let rivendell = key(&w.rivendell);
        let id = w.rivendell.device.id();
        let try_length = |length| {
            scopes.create_segment(
                &flags,
                &put(&w.id, &rivendell, 1, length),
                &id,
                &|| Ok(None),
                &mut || Created::New,
            )
        };
        assert_eq!(try_length(MIB + 1), Err(Refusal::TooLarge));
        assert_eq!(try_length(MIB), Ok(Created::New));
        assert_eq!(segment_max(&flags), MIB);
    }

    #[test]
    fn the_next_manifest_is_judged_by_its_key_before_its_body() {
        let w = world("next_key", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 1);
        let stranger = Device::from_seeds("x", &[77; 32], &[78; 32]).sign.public();
        let garbage = b"not a manifest";
        assert_eq!(
            manifest_n(&scopes, &w, &stranger, 2, garbage),
            Err(Refusal::NotAdmitted)
        );
        let result = manifest_n(&scopes, &w, &key(&w.rivendell), 2, garbage);
        assert!(matches!(result, Err(Refusal::Manifest(_))), "{result:?}");
        let by_owner = manifest_n(&scopes, &w, &w.owner.sign.public(), 2, garbage);
        assert!(
            matches!(by_owner, Err(Refusal::Manifest(_))),
            "{by_owner:?}"
        );
    }

    #[test]
    fn a_manifest_above_one_mebibyte_is_too_large() {
        let w = world("manifest_cap", 10);
        let scopes = scopes(&[&w.owner]);
        let signer = key(&w.rivendell);
        let big = put(&w.id, &signer, 1, MANIFEST_MAX + 1);
        let result = scopes.create_manifest(&flags(), &big, &w.files[0], &mut || {
            panic!("linked a big manifest")
        });
        assert_eq!(result, Err(Refusal::TooLarge));
    }

    #[test]
    fn scopes_are_counted_per_owner() {
        let (a, b) = (world("count_a", 10), world("count_b", 40));
        let a2 = world("count_a2", 10);
        let scopes = scopes(&[&a.owner, &b.owner]);
        let flags = Flags {
            max_scopes: 1,
            ..flags()
        };
        let first = |w: &World| {
            scopes.create_manifest(
                &flags,
                &put(&w.id, &key(&w.rivendell), 1, w.files[0].len() as u64),
                &w.files[0],
                &mut || Created::New,
            )
        };
        assert_eq!(first(&a), Ok(Created::New));
        assert_eq!(first(&a2), Err(Refusal::Quota));
        assert!(scopes.get(&a2.id).is_none());
        assert_eq!(first(&b), Ok(Created::New));
        assert_eq!(
            first(&a),
            Ok(Created::Same),
            "a held scope is not counted again"
        );
    }

    #[test]
    fn a_scope_that_is_not_valid_does_not_count() {
        let (a, a2) = (world("count_invalid", 10), world("count_invalid2", 10));
        let scopes = scopes(&[&a.owner]);
        let flags = Flags {
            max_scopes: 1,
            ..flags()
        };
        let create = |w: &World| {
            scopes.create_manifest(
                &flags,
                &put(&w.id, &key(&w.rivendell), 1, w.files[0].len() as u64),
                &w.files[0],
                &mut || Created::New,
            )
        };
        assert_eq!(create(&a), Ok(Created::New));
        locked(&scopes.get(&a.id).unwrap()).standing = Standing::Invalid("broken".into());
        assert_eq!(create(&a2), Ok(Created::New));
    }

    #[test]
    fn reservations_count_against_the_scope_cap() {
        let w = world("reserve", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 1);
        let flags = Flags {
            max_scope_mb: 1,
            ..flags()
        };
        let stored = w.files[0].len() as u64;
        let half = MIB / 2 - stored;
        assert_eq!(scopes.reserve(&flags, &w.id, half), Ok(half));
        assert_eq!(
            scopes.reserve(&flags, &w.id, MIB / 2 + 1),
            Err(Refusal::Quota)
        );
        assert_eq!(scopes.reserve(&flags, &w.id, MIB / 2), Ok(MIB / 2));
        assert_eq!(status(&scopes, &w.id), (stored, half + MIB / 2));
        assert_eq!(scopes.reserve(&flags, &w.id, 1), Err(Refusal::Quota));
        scopes.release(&w.id, MIB / 2);
        assert_eq!(status(&scopes, &w.id).1, half);
        assert_eq!(scopes.reserve(&flags, &w.id, MIB / 2), Ok(MIB / 2));
    }

    #[test]
    fn a_create_gives_its_reservation_back() {
        let w = world("reserve_create", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let flags = Flags {
            max_scope_mb: 1,
            ..flags()
        };
        let (stored, _) = status(&scopes, &w.id);
        let rivendell = key(&w.rivendell);
        let id = w.rivendell.device.id();
        let body = MIB / 4;
        let reserved = scopes.reserve(&flags, &w.id, body).unwrap();
        let create = |seq, reserved, outcome: Created| {
            scopes.create_segment(
                &flags,
                &Put {
                    reserved,
                    ..put(&w.id, &rivendell, seq, body)
                },
                &id,
                &|| Ok(None),
                &mut || outcome.clone_for_test(),
            )
        };
        assert_eq!(
            create(1, reserved, Created::Failed("io".into())),
            Ok(Created::Failed("io".into()))
        );
        assert_eq!(status(&scopes, &w.id), (stored, 0));
        let reserved = scopes.reserve(&flags, &w.id, body).unwrap();
        assert_eq!(create(1, reserved, Created::New), Ok(Created::New));
        assert_eq!(status(&scopes, &w.id), (stored + body, 0));
        let reserved = scopes.reserve(&flags, &w.id, body).unwrap();
        assert_eq!(
            create(3, reserved, Created::New),
            Err(Refusal::NotNext),
            "a refusal gives it back as well"
        );
        assert_eq!(status(&scopes, &w.id), (stored + body, 0));
    }

    #[test]
    fn two_uploads_in_flight_cannot_overshoot_the_cap() {
        let w = world("in_flight", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 3);
        let flags = Flags {
            max_scope_mb: 1,
            ..flags()
        };
        let (stored, _) = status(&scopes, &w.id);
        let body = (MIB - stored) / 2 + 1;
        assert_eq!(scopes.reserve(&flags, &w.id, body), Ok(body));
        assert_eq!(scopes.reserve(&flags, &w.id, body), Err(Refusal::Quota));
    }

    #[test]
    fn a_scope_the_relay_does_not_hold_books_nothing() {
        let w = world("reserve_unheld", 10);
        let scopes = scopes(&[&w.owner]);
        assert_eq!(scopes.reserve(&flags(), &w.id, 5), Ok(0));
        scopes.release(&w.id, 5);
    }

    #[test]
    fn the_owner_lists_its_valid_scopes_sorted() {
        let (a, b) = (world("list_a", 10), world("list_b", 40));
        let a2 = world("list_a2", 10);
        let scopes = scopes(&[&a.owner, &b.owner]);
        for w in [&a, &b, &a2] {
            let created = manifest_n(&scopes, w, &key(&w.rivendell), 1, &w.files[0]);
            assert_eq!(created, Ok(Created::New));
        }
        let mut want = vec![a.id.clone(), a2.id.clone()];
        want.sort();
        assert_eq!(scopes.owner_scopes(&a.owner.sign.public()), Ok(want));
        assert_eq!(
            scopes.owner_scopes(&b.owner.sign.public()),
            Ok(vec![b.id.clone()])
        );
        locked(&scopes.get(&a.id).unwrap()).standing = Standing::Invalid("x".into());
        assert_eq!(
            scopes.owner_scopes(&a.owner.sign.public()),
            Ok(vec![a2.id.clone()])
        );
        assert_eq!(
            scopes.owner_scopes(&key(&a.rivendell)),
            Err(Refusal::NotAdmitted)
        );
        let unadmitted = Owner::derive(&[1; 16]).sign.public();
        assert_eq!(scopes.owner_scopes(&unadmitted), Err(Refusal::NotAdmitted));
    }

    #[test]
    fn only_a_device_of_a_valid_admitted_scope_is_enrolled() {
        let w = world("enrolled", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 2);
        let (rivendell, bagend) = (key(&w.rivendell), key(&w.bagend));
        assert!(scopes.enrolled(&rivendell));
        assert!(scopes.enrolled(&bagend));
        assert!(!scopes.enrolled(&w.owner.sign.public()));
        assert!(!scopes.enrolled(&[3; 32]));
        let created = manifest_n(&scopes, &w, &rivendell, 3, &w.files[2]);
        assert_eq!(created, Ok(Created::New));
        assert!(!scopes.enrolled(&bagend), "revoked in the latest version");
        locked(&scopes.get(&w.id).unwrap()).standing = Standing::NotAdmitted;
        assert!(!scopes.enrolled(&rivendell));
    }

    #[test]
    fn an_owner_dropped_from_the_flags_enrolls_nobody() {
        let w = world("owner_dropped", 10);
        let scopes = scopes(&[&w.owner]);
        admit_to(&scopes, &w, 1);
        let none = Scopes {
            owners: Vec::new(),
            held: Mutex::new(scopes.map().clone()),
        };
        assert!(!none.enrolled(&key(&w.rivendell)));
    }

    impl Created {
        fn clone_for_test(&self) -> Created {
            match self {
                Created::New => Created::New,
                Created::Same => Created::Same,
                Created::Other => Created::Other,
                Created::Full(s) => Created::Full(s.clone()),
                Created::Failed(s) => Created::Failed(s.clone()),
            }
        }
    }
}
