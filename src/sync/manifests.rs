//! One scope's manifests on the transport: publishing, confirming, adopting and losing versions, the pin, and the
//! changes this device did not make.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::identity::keys::{self, Identity};
use crate::identity::manifest::{self, Known, Manifest, Recipient, Scope};
use crate::shared::{hash, store};
use crate::sync::scopes;
use crate::sync::transport::{self, Put, Transport};

/// How long a `file://` version that changes the epoch must stand before it is confirmed.
const EPOCH_WAIT: jiff::SignedDuration = jiff::SignedDuration::from_mins(10);
/// How long a change stays listed.
const KEEP_DAYS: i64 = 24 * 30;
/// Who signed a version, while manifests name no writer.
const SIGNER: &str = "owner key";
/// How many times a cycle moves a lost version aside and goes on.
const ROUNDS: usize = 4;

/// What one cycle's step needs to know.
pub struct Input<'a> {
    pub root: &'a Path,
    /// The scope's config name.
    pub name: &'a str,
    /// The config URL.
    pub url: &'a str,
    pub identity: &'a Identity,
    pub now: jiff::Timestamp,
}

/// Why a scope does not sync this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The config URL does not match the manifest's `transport`.
    Pin,
    /// The transport holds another version than one this device confirmed.
    Differs,
    /// The latest version no longer lists this device.
    Removed,
    /// No scope of this name lists this device here, or its version 1 may not be published.
    NotInScope,
    /// A folder that holds none of this device's confirmed versions.
    NoScopes,
    /// A scope to restore whose epoch change has not stood long enough on the folder.
    Settling,
}

/// A scope that may not sync, with the line to print when it changes.
#[derive(Debug, PartialEq)]
pub struct Stop {
    pub reason: Reason,
    pub line: String,
}

/// What one cycle's step did for a scope name.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Lines to print now, each once: changes made elsewhere, a lost fork, a version that does not verify, a scope
    /// that replaces another.
    pub events: Vec<String>,
    /// The scope id to sync under the name this cycle, `None` when `stop` says why not.
    pub scope: Option<String>,
    pub stop: Option<Stop>,
    /// The transport's message when it refused a version because it is full. The scope still syncs.
    pub full: Option<String>,
    /// The scopes of this name that `scope` replaces: their notes go into it.
    pub replaced: Vec<String>,
    /// Why a scope could not be read or written this cycle, such as a folder that is not there. What the other
    /// scopes did is in the fields above.
    pub error: Option<String>,
}

/// One cycle for the scope the config names `input.name`, among the scopes already in the store: publishes pending
/// versions, confirms them, moves a lost fork aside, adopts what other devices wrote, and decides whether the scope
/// may sync. It writes no version except what `manifest::lose` applies again, and takes no scope id from the
/// transport, except to restore the one scope of this name that lists this device when the store holds none. `Err`
/// is a failure before any scope was looked at; one scope's failure is `Outcome::error`.
pub fn step(t: &dyn Transport, input: &Input) -> Result<Outcome, String> {
    let owner = input.identity.owner.sign.public();
    let who = Recipient::device(&input.identity.device);
    let known = manifest::survey(input.root, Some(&owner), Some(&who))?;
    let mine: Vec<&Known> = known
        .iter()
        .filter(|k| k.mine && k.last_name.as_deref() == Some(input.name))
        .collect();
    let mut ids: Vec<String> = mine.iter().map(|k| k.scope.id.clone()).collect();
    let mut outcome = Outcome::default();
    if ids.is_empty() {
        match restore(t, input, &owner, &who) {
            Restored::Scope { id, devices } => {
                outcome.events.push(format!(
                    "sync {}: resumed scope {id} from the folder ({devices} devices)",
                    input.name
                ));
                ids.push(id);
            }
            Restored::Stop(stop) => {
                outcome.stop = Some(stop);
                return Ok(outcome);
            }
            Restored::Error(why) => {
                outcome.error = Some(why);
                return Ok(outcome);
            }
        }
    }
    let mut ones: Vec<One> = ids.iter().map(|id| one(t, input, &who, id)).collect();
    let live: Vec<usize> = (0..ones.len())
        .filter(|i| ones[*i].stop.is_none() && ones[*i].error.is_none())
        .collect();
    let mut chosen = live.first().copied();
    if live.len() > 1 {
        let ids: Vec<&str> = live.iter().map(|i| ones[*i].id.as_str()).collect();
        let mut listing = match scopes::list(t, &owner, &who) {
            Ok(listing) => listing,
            Err(why) => {
                outcome.error = Some(why);
                scopes::Listing::default()
            }
        };
        listing.found.retain(|f| ids.contains(&f.id.as_str()));
        if let Some(pick) = scopes::pick(&listing.found, input.name) {
            let (winner, rivals) = (pick.chosen.id.clone(), pick.rivals);
            for rival in rivals {
                let line = format!("sync {}: scope {winner} replaces {}", input.name, rival.id);
                if let Some(one) = ones.iter_mut().find(|o| o.id == rival.id) {
                    one.say(line);
                }
                outcome.replaced.push(rival.id.clone());
            }
            chosen = live.iter().copied().find(|i| ones[*i].id == winner);
        }
    }
    outcome.scope = chosen.map(|i| ones[i].id.clone());
    for one in &mut ones {
        outcome.events.extend(settle(input, one)?);
        outcome.full = outcome.full.or(one.full.take());
        outcome.error = outcome.error.or(one.error.take());
    }
    if chosen.is_none() {
        outcome.stop = ones.into_iter().next().and_then(|o| o.stop);
    }
    Ok(outcome)
}

/// A scope this step looked at.
struct One {
    id: String,
    /// What the file held when the step began.
    before: State,
    state: State,
    /// Lines to print now.
    events: Vec<String>,
    /// Lines that hold while a condition does, printed once until it ends.
    conditions: Vec<String>,
    stop: Option<Stop>,
    full: Option<String>,
    error: Option<String>,
}

impl One {
    fn say(&mut self, line: String) {
        if !self.conditions.contains(&line) {
            self.conditions.push(line);
        }
    }
}

/// What one scope did; a failure is kept in `error`, so what it did before it is still reported.
fn one(t: &dyn Transport, input: &Input, who: &Recipient, id: &str) -> One {
    let state = read_state(input.root, id);
    let mut one = One {
        id: id.to_string(),
        before: state.clone(),
        state,
        events: Vec::new(),
        conditions: Vec::new(),
        stop: None,
        full: None,
        error: None,
    };
    if let Err(why) = run(t, input, who, &mut one) {
        one.error = Some(why);
    }
    one
}

fn run(t: &dyn Transport, input: &Input, who: &Recipient, one: &mut One) -> Result<(), String> {
    let id = one.id.clone();
    let local = manifest::read_scope(input.root, &id)?;
    if let Some(invalid) = &local.invalid {
        return Err(invalid.to_string());
    }
    one.stop = pin(input, &local);
    if one.stop.is_some() {
        return Ok(());
    }
    t.reachable()?;
    one.stop = folder(t, input, &local, one)?;
    if one.stop.is_some() {
        return Ok(());
    }
    let mut lock = None;
    for _ in 0..ROUNDS {
        if !pending(t, input, who, one, &mut lock)? {
            break;
        }
    }
    drop(lock);
    if one.stop.is_some() {
        return Ok(());
    }
    let mut lock = None;
    adopt(t, input, who, one, &mut lock)?;
    let local = manifest::read_scope(input.root, &id)?;
    let reading = manifest::read(&local, who).map_err(|i| i.to_string())?;
    one.stop = if reading.opened.is_none() {
        Some(Stop {
            reason: Reason::Removed,
            line: format!(
                "sync {}: this device was removed from the scope",
                input.name
            ),
        })
    } else {
        pin(input, &local)
    };
    Ok(())
}

fn not_in_scope(name: &str) -> Stop {
    Stop {
        reason: Reason::NotInScope,
        line: format!(
            "sync {name}: this device is not in the scope; run bilbo device recover on this device"
        ),
    }
}

/// The pin check, against the latest local version.
fn pin(input: &Input, local: &Scope) -> Option<Stop> {
    let pinned = &local.latest()?.manifest.transport;
    (!manifest::transport_matches(pinned, input.url)).then(|| Stop {
        reason: Reason::Pin,
        line: format!(
            "sync {}: the manifest pins {pinned}, the config says {}; run bilbo device init to move the scope, or set the config back",
            input.name, input.url
        ),
    })
}

/// A folder that holds none of this scope's confirmed versions, or another version than one it confirmed.
fn folder(
    t: &dyn Transport,
    input: &Input,
    local: &Scope,
    one: &mut One,
) -> Result<Option<Stop>, String> {
    let name = input.name;
    let confirmed: Vec<_> = local
        .versions
        .iter()
        .filter(|v| !local.pending.contains(&v.manifest.n))
        .collect();
    if t.keeps() || confirmed.is_empty() {
        return Ok(None);
    }
    let mut present = false;
    let mut missing = Vec::new();
    for v in confirmed {
        let n = v.manifest.n;
        match t.get(&transport::manifest_path(&local.id, n))? {
            None => missing.push(v),
            Some(bytes) if bytes == v.bytes => present = true,
            Some(_) => {
                return Ok(Some(Stop {
                    reason: Reason::Differs,
                    line: format!(
                        "sync {name}: manifest {n} on the transport differs from the confirmed one; run bilbo device to compare"
                    ),
                }));
            }
        }
    }
    if present {
        for v in missing {
            match t.create(&transport::manifest_path(&local.id, v.manifest.n), &v.bytes) {
                Put::Created | Put::Exists => {}
                Put::Full(message) => one.full = Some(message),
                Put::Unreachable(reason) => return Err(reason),
            }
        }
    }
    let path = folder_path(input);
    Ok((!present).then(|| Stop {
        reason: Reason::NoScopes,
        line: format!("sync {name}: {path} holds none of this device's scopes; if the folder moved, change scope.{name}.sync on every device"),
    }))
}

/// The config URL without its scheme.
fn folder_path<'a>(input: &Input<'a>) -> &'a str {
    input.url.strip_prefix("file://").unwrap_or(input.url)
}

fn locked<'a>(
    slot: &'a mut Option<manifest::Lock>,
    root: &Path,
) -> Result<&'a manifest::Lock, String> {
    if slot.is_none() {
        *slot = Some(manifest::lock(root)?);
    }
    Ok(slot.as_ref().expect("taken above"))
}

/// Whether version `n`, which the transport holds with this device's bytes, may be confirmed now: at once for version
/// 1, a version that keeps the epoch and a transport that keeps what it stores, else 10 minutes after the write.
fn settled(t: &dyn Transport, input: &Input, local: &Scope, state: &State, n: u64) -> bool {
    if n == 1 || t.keeps() {
        return true;
    }
    let epoch = |k: u64| local.versions[(k - 1) as usize].manifest.epoch;
    if epoch(n) == epoch(n - 1) {
        return true;
    }
    state
        .published
        .get(&n)
        .is_some_and(|at| input.now.as_second() - at >= EPOCH_WAIT.as_secs())
}

/// Publishes and confirms the pending versions in order. `true` when a fork was moved aside and another round should
/// publish what was written again.
fn pending(
    t: &dyn Transport,
    input: &Input,
    who: &Recipient,
    one: &mut One,
    lock: &mut Option<manifest::Lock>,
) -> Result<bool, String> {
    let id = one.id.clone();
    let local = manifest::read_scope(input.root, &id)?;
    for &n in &local.pending {
        let v = &local.versions[(n - 1) as usize];
        let path = transport::manifest_path(&id, n);
        let mut held = t.get(&path)?;
        if held.is_none() {
            if n == 1
                && let Some(stop) = blocked(t, input, who)?
            {
                one.stop = Some(stop);
                return Ok(false);
            }
            match t.create(&path, &v.bytes) {
                Put::Created | Put::Exists => held = t.get(&path)?,
                Put::Full(message) => {
                    one.full = Some(message);
                    return Ok(false);
                }
                Put::Unreachable(reason) => return Err(reason),
            }
        }
        match held {
            None => return Ok(false),
            Some(bytes) if bytes == v.bytes => {
                one.state
                    .published
                    .entry(n)
                    .or_insert(input.now.as_second());
                if !settled(t, input, &local, &one.state, n) {
                    return Ok(false);
                }
                if manifest::confirm(locked(lock, input.root)?, &id, n, &bytes)? {
                    one.state.published.remove(&n);
                } else {
                    return Ok(false);
                }
            }
            Some(bytes) => return fork(t, input, who, one, lock, &local, (n, &bytes)),
        }
    }
    Ok(false)
}

/// Why version 1 may not go on the transport now: this device is in no scope there while the owner has one it cannot
/// open, the transport holds a scope of this name that this device opens, or a scope whose version 1 does not verify,
/// which may be one still arriving. A version 1 beside any of them would hide it or fork it.
fn blocked(t: &dyn Transport, input: &Input, who: &Recipient) -> Result<Option<Stop>, String> {
    let owner = input.identity.owner.sign.public();
    let listing = scopes::list(t, &owner, who)?;
    if listing.outsider() || scopes::pick(&listing.found, input.name).is_some() {
        return Ok(Some(not_in_scope(input.name)));
    }
    Ok(unattributed(input, &listing))
}

/// The stop for the first scope of the listing whose version 1 is missing or does not verify.
fn unattributed(input: &Input, listing: &scopes::Listing) -> Option<Stop> {
    let (name, path) = (input.name, folder_path(input));
    listing.unattributed.first().map(|(id, why)| Stop {
        reason: Reason::NotInScope,
        line: format!("sync {name}: {path} holds scope {id} that does not verify: {why}"),
    })
}

/// The transport holds `bytes` as version `n`, not this device's pending one.
fn fork(
    t: &dyn Transport,
    input: &Input,
    who: &Recipient,
    one: &mut One,
    lock: &mut Option<manifest::Lock>,
    local: &Scope,
    (n, bytes): (u64, &[u8]),
) -> Result<bool, String> {
    let name = input.name;
    let chain = scopes::chain(t, &one.id)?;
    let Some(winner) = chain
        .versions
        .get((n - 1) as usize)
        .filter(|w| w.bytes == bytes)
    else {
        let why = chain.invalid.as_ref().map_or_else(
            || format!("manifest/{n}.json on the transport is not a valid version"),
            |i| i.to_string(),
        );
        one.say(format!("sync {name}: {why}"));
        return Ok(false);
    };
    if let Err(invalid) = manifest::read(&prefix(&one.id, &chain, n), who) {
        one.say(format!("sync {name}: {invalid}"));
        return Ok(false);
    }
    if !ready(t, input, &mut one.state, &chain, n) {
        return Ok(false);
    }
    let before = (n > 1).then(|| local.versions[(n - 2) as usize].manifest.clone());
    let lost = manifest::lose(locked(lock, input.root)?, &one.id, n, bytes, input.identity)?;
    one.events.push(format!(
        "sync {name}: manifest {n} was written elsewhere first; this device's version is kept in manifest/lost and its changes are written again"
    ));
    for why in lost.skipped.into_iter().chain(lost.problem) {
        one.events.push(format!("sync {name}: {why}"));
    }
    one.state.published.retain(|k, _| *k < n);
    one.state.seen.retain(|k, _| *k > n);
    if let Some(before) = before {
        let found = changes(&before, &winner.manifest, n);
        record(input, &one.id, &found)?;
        one.events.extend(found.iter().map(|c| c.line(name)));
    }
    Ok(true)
}

/// The transport's versions 1 through `n`, as a scope to check as a member.
fn prefix(id: &str, chain: &Scope, n: u64) -> Scope {
    let files: Vec<Vec<u8>> = chain.versions[..n as usize]
        .iter()
        .map(|v| v.bytes.clone())
        .collect();
    manifest::verify_scope(id, &files)
}

/// Whether version `n` of `chain`, written elsewhere, may be copied in now. On a folder a version that changes the
/// epoch waits until it has been read unchanged for 10 minutes, so no device seals under an epoch the folder has not
/// settled.
fn ready(t: &dyn Transport, input: &Input, state: &mut State, chain: &Scope, n: u64) -> bool {
    if t.keeps() || n < 2 {
        return true;
    }
    let epoch = |k: u64| chain.versions[(k - 1) as usize].manifest.epoch;
    if epoch(n) == epoch(n - 1) {
        return true;
    }
    let hash = hash::sha256_hex(&chain.versions[(n - 1) as usize].bytes);
    let now = input.now.as_second();
    match state.seen.get(&n) {
        Some((seen, at)) if *seen == hash => now - at >= EPOCH_WAIT.as_secs(),
        _ => {
            state.seen.insert(n, (hash, now));
            false
        }
    }
}

/// Copies in the versions the transport holds beyond this device's, each valid under the manifest rules and its
/// chain check by this device.
fn adopt(
    t: &dyn Transport,
    input: &Input,
    who: &Recipient,
    one: &mut One,
    lock: &mut Option<manifest::Lock>,
) -> Result<(), String> {
    let (name, id) = (input.name, one.id.clone());
    let local = manifest::read_scope(input.root, &id)?;
    let chain = scopes::chain(t, &id)?;
    let mut limit = chain.versions.len() as u64;
    if let Some(invalid) = &chain.invalid {
        one.say(format!("sync {name}: {invalid}"));
    }
    if let Err(invalid) = manifest::read(&chain, who) {
        one.say(format!("sync {name}: {invalid}"));
        limit = limit.min(invalid.n.saturating_sub(1));
    }
    if !local.pending.is_empty() {
        return Ok(());
    }
    let have = local.versions.len() as u64;
    let mut last = have;
    for n in have + 1..=limit {
        if !ready(t, input, &mut one.state, &chain, n) {
            break;
        }
        let (before, after) = (
            &chain.versions[(n - 2) as usize],
            &chain.versions[(n - 1) as usize],
        );
        let found = changes(&before.manifest, &after.manifest, n);
        record(input, &id, &found)?;
        manifest::adopt(locked(lock, input.root)?, &id, n, &after.bytes)?;
        one.events.extend(found.iter().map(|c| c.line(name)));
        last = n;
    }
    one.state.seen.retain(|k, _| *k > last);
    Ok(())
}

/// Reports the conditions that are new, and keeps the step's state.
fn settle(input: &Input, one: &mut One) -> Result<Vec<String>, String> {
    let mut lines = std::mem::take(&mut one.events);
    lines.extend(
        one.conditions
            .iter()
            .filter(|c| !one.state.said.contains(c))
            .cloned(),
    );
    one.state.said = std::mem::take(&mut one.conditions);
    if one.state != one.before {
        write_state(input.root, &one.id, &one.state)?;
    }
    record(input, &one.id, &[])?;
    Ok(lines)
}

/// What `restore` decided.
enum Restored {
    Scope { id: String, devices: usize },
    Stop(Stop),
    Error(String),
}

/// A scope of the owner on the transport that opens to the config name for this device on its member-valid prefix.
struct Candidate {
    id: String,
    chain: Scope,
    /// How many versions the prefix holds.
    limit: u64,
}

/// The store holds no manifest of this name: copies in the one scope of that name on the transport that lists this
/// device, as the epoch rule allows now (the rest comes through `adopt`). Every scope of the owner counts, whether or
/// not its whole chain verifies, since a scope with a bad version is still the scope; with several candidates it
/// copies none, and it copies nothing while a scope whose version 1 does not verify may be the one meant.
fn restore(t: &dyn Transport, input: &Input, owner: &[u8; 32], who: &Recipient) -> Restored {
    match restore_scope(t, input, owner, who) {
        Ok(restored) => restored,
        Err(why) => Restored::Error(why),
    }
}

fn restore_scope(
    t: &dyn Transport,
    input: &Input,
    owner: &[u8; 32],
    who: &Recipient,
) -> Result<Restored, String> {
    t.reachable()?;
    let name = input.name;
    let listing = scopes::list(t, owner, who)?;
    if let Some(stop) = unattributed(input, &listing) {
        return Ok(Restored::Stop(stop));
    }
    let ids = listing.found.iter().map(|f| &f.id).chain(&listing.closed);
    let mut candidates = Vec::new();
    for id in ids {
        let chain = scopes::chain(t, id)?;
        let mut limit = chain.versions.len() as u64;
        if let Err(invalid) = manifest::read(&chain, who) {
            limit = invalid.n.saturating_sub(1);
        }
        let opened = manifest::open(&prefix(id, &chain, limit), who);
        if opened.ok().flatten().is_some_and(|o| o.name == name) {
            candidates.push(Candidate {
                id: id.clone(),
                chain,
                limit,
            });
        }
    }
    let candidate = match candidates.as_slice() {
        [] => return Ok(Restored::Stop(not_in_scope(name))),
        [candidate] => candidate,
        _ => {
            return Ok(Restored::Stop(Stop {
                reason: Reason::NotInScope,
                line: format!(
                    "sync {name}: the folder holds several scopes named {name}; run bilbo device recover on this device"
                ),
            }));
        }
    };
    let (id, chain) = (&candidate.id, &candidate.chain);
    let mut state = read_state(input.root, id);
    let mut allowed = candidate.limit;
    for n in 2..=candidate.limit {
        if !ready(t, input, &mut state, chain, n) {
            allowed = n - 1;
            break;
        }
    }
    let copied = prefix(id, chain, allowed);
    if manifest::open(&copied, who).ok().flatten().is_none() {
        fs::create_dir_all(scope_dir(input.root, id))
            .map_err(|e| format!("cannot create {}: {e}", scope_dir(input.root, id).display()))?;
        write_state(input.root, id, &state)?;
        return Ok(Restored::Stop(Stop {
            reason: Reason::Settling,
            line: format!("sync {name}: waiting for the folder to settle scope {id}"),
        }));
    }
    let lock = manifest::lock(input.root)?;
    let held = manifest::read_scope(input.root, id)?;
    if let Some(invalid) = &held.invalid {
        return Err(invalid.to_string());
    }
    for v in copied.versions.iter().skip(held.versions.len()) {
        manifest::adopt(&lock, id, v.manifest.n, &v.bytes)?;
    }
    drop(lock);
    state.seen.retain(|k, _| *k > allowed);
    write_state(input.root, id, &state)?;
    let devices = copied.latest().map_or(0, |v| v.manifest.devices.len());
    Ok(Restored::Scope {
        id: id.clone(),
        devices,
    })
}

/// What this step keeps beside `state.json`, in `manifests.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct State {
    /// When this device put each pending version on the transport, in seconds since the epoch.
    #[serde(default)]
    published: BTreeMap<u64, i64>,
    /// When this device first read each version written elsewhere that changes the epoch, with the hash of its
    /// bytes, in seconds since the epoch.
    #[serde(default)]
    seen: BTreeMap<u64, (String, i64)>,
    /// The condition lines already printed, for as long as the conditions hold.
    #[serde(default)]
    said: Vec<String>,
}

fn scope_dir(root: &Path, id: &str) -> PathBuf {
    store::scopes_dir(root).join(id)
}

/// The state, empty when the file is missing or damaged: a lost write time only delays a confirmation.
fn read_state(root: &Path, id: &str) -> State {
    fs::read(scope_dir(root, id).join("manifests.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_state(root: &Path, id: &str, state: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec(state).expect("the state serializes");
    write_whole(&scope_dir(root, id).join("manifests.json"), &bytes)
}

/// Writes `path` whole through a hidden `.tmp-<random>` beside it.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let random = keys::random::<8>()?;
    let temporary = path.with_file_name(format!(".tmp-{}", keys::hex(&random)));
    let fail = |e: std::io::Error| {
        let _ = fs::remove_file(&temporary);
        format!("cannot write {}: {e}", path.display())
    };
    fs::File::create(&temporary)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(fail)
}

/// What a version from another device changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Device,
    Epoch,
}

/// One line of `changes.jsonl`: a device added or an epoch started by a version this device did not write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    /// RFC 3339, when this device adopted the version.
    pub at: String,
    /// The manifest version.
    pub n: u64,
    pub kind: Kind,
    /// The added device's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub signer: String,
}

impl Change {
    fn line(&self, name: &str) -> String {
        let n = self.n;
        match (&self.kind, &self.device) {
            (Kind::Device, Some(device)) => format!(
                "sync {name}: device {device} added by {} (manifest {n})",
                self.signer
            ),
            _ => format!("sync {name}: epoch changed (manifest {n})"),
        }
    }

    fn same(&self, other: &Change) -> bool {
        (self.n, self.kind, &self.device) == (other.n, other.kind, &other.device)
    }
}

/// The devices `after` adds to `before`, and its epoch when it starts a new one.
fn changes(before: &Manifest, after: &Manifest, n: u64) -> Vec<Change> {
    let change = |kind, device| Change {
        at: String::new(),
        n,
        kind,
        device,
        signer: SIGNER.to_string(),
    };
    let mut all: Vec<Change> = after
        .devices
        .iter()
        .filter(|d| !before.lists(&d.id))
        .map(|d| change(Kind::Device, Some(d.name.clone())))
        .collect();
    if after.epoch != before.epoch {
        all.push(change(Kind::Epoch, None));
    }
    all
}

fn changes_path(root: &Path, id: &str) -> PathBuf {
    scope_dir(root, id).join("changes.jsonl")
}

fn cutoff(now: jiff::Timestamp) -> Result<jiff::Timestamp, String> {
    now.checked_sub(jiff::SignedDuration::from_hours(KEEP_DAYS))
        .map_err(|e| format!("cannot compute the cutoff of the changes: {e}"))
}

/// The changes of scope `id` from the last 30 days, oldest first. It reads and changes nothing.
pub fn recent(root: &Path, id: &str, now: jiff::Timestamp) -> Result<Vec<Change>, String> {
    let path = changes_path(root, id);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let cutoff = cutoff(now)?;
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Change>(line).ok())
        .filter(|c| c.at.parse::<jiff::Timestamp>().is_ok_and(|at| at >= cutoff))
        .collect())
}

/// Appends `added`, stamped now, to `changes.jsonl` unless an equal line is there, and drops the lines past 30 days.
fn record(input: &Input, id: &str, added: &[Change]) -> Result<(), String> {
    let path = changes_path(input.root, id);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let cutoff = cutoff(input.now)?;
    let (mut lines, mut held, mut dropped) = (Vec::new(), Vec::new(), false);
    for line in text.lines().filter(|l| !l.is_empty()) {
        match serde_json::from_str::<Change>(line) {
            Ok(c) if c.at.parse::<jiff::Timestamp>().is_ok_and(|at| at < cutoff) => dropped = true,
            Ok(c) => {
                held.push(c);
                lines.push(line.to_string());
            }
            Err(_) => lines.push(line.to_string()),
        }
    }
    let at = input.now.to_string();
    let fresh: Vec<Change> = added
        .iter()
        .filter(|c| !held.iter().any(|h| h.same(c)))
        .map(|c| Change {
            at: at.clone(),
            ..c.clone()
        })
        .collect();
    if fresh.is_empty() && !dropped {
        return Ok(());
    }
    lines.extend(
        fresh
            .iter()
            .map(|c| serde_json::to_string(c).expect("a change serializes")),
    );
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    write_whole(&path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keys::{Device, Identity, Owner};
    use crate::identity::manifest::Link;
    use crate::identity::manifest::Member;
    use crate::sync::transport::Folder;
    use std::time::SystemTime;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("bilbo-manifests-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("folder")).unwrap();
        Scratch(dir)
    }

    const T0: i64 = 1_800_000_000;

    fn at(secs: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(T0 + secs).unwrap()
    }

    fn identity(owner: u8, name: &str, seed: u8) -> Identity {
        Identity {
            owner: Owner::derive(&[owner; 16]).file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    fn a() -> Identity {
        identity(0, "a", 1)
    }

    fn b() -> Identity {
        identity(0, "b", 3)
    }

    fn c() -> Identity {
        identity(0, "c", 5)
    }

    fn moria() -> Identity {
        identity(0, "moria", 7)
    }

    fn member(who: &Identity) -> Member {
        Member::of(&who.device)
    }

    fn store(d: &Scratch, name: &str) -> PathBuf {
        let dir = d.0.join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn folder_path(d: &Scratch) -> PathBuf {
        d.0.join("folder")
    }

    fn url_of(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    /// Writes version 1 of `name` pinned to `file://`, pending.
    fn make(root: &Path, who: &Identity, name: &str, others: &[&Identity]) -> String {
        make_pinned(root, who, name, "file://", others)
    }

    fn make_pinned(
        root: &Path,
        who: &Identity,
        name: &str,
        url: &str,
        others: &[&Identity],
    ) -> String {
        let members: Vec<Member> = others.iter().map(|i| member(i)).collect();
        let lock = manifest::lock(root).unwrap();
        manifest::create(&lock, who, name, url, &members)
            .unwrap()
            .scope
    }

    fn add(root: &Path, who: &Identity, id: &str, target: &Identity) {
        let lock = manifest::lock(root).unwrap();
        let scope = manifest::read_scope(root, id).unwrap();
        let opened = manifest::open(&scope, &Recipient::device(&who.device))
            .unwrap()
            .unwrap();
        manifest::add_device(
            &lock,
            &scope,
            &opened.keys[&opened.epoch],
            &member(target),
            who,
        )
        .unwrap();
    }

    fn revoke(root: &Path, who: &Identity, id: &str, target: &Identity) {
        let lock = manifest::lock(root).unwrap();
        let scope = manifest::read_scope(root, id).unwrap();
        let opened = manifest::open(&scope, &Recipient::device(&who.device))
            .unwrap()
            .unwrap();
        manifest::revoke(&lock, &scope, &opened, who, &target.device.id()).unwrap();
    }

    /// Copies every version of `id` from one store into another as confirmed, as `recover` or the wizard would.
    fn copy_in(to: &Path, from: &Path, id: &str) {
        let lock = manifest::lock(to).unwrap();
        for v in manifest::read_scope(from, id).unwrap().versions {
            manifest::adopt(&lock, id, v.manifest.n, &v.bytes).unwrap();
        }
    }

    fn folder_of(d: &Scratch, who: &Identity) -> Folder {
        Folder::new(folder_path(d), &who.device.id())
    }

    /// Puts every version of `id` on the folder, as another device's watch would have.
    fn put(d: &Scratch, root: &Path, who: &Identity, id: &str) {
        let t = folder_of(d, who);
        for v in local(root, id).versions {
            let path = transport::manifest_path(id, v.manifest.n);
            assert_eq!(t.create(&path, &v.bytes), Put::Created);
        }
    }

    fn go(d: &Scratch, root: &Path, who: &Identity, name: &str, secs: i64) -> Outcome {
        let url = url_of(&folder_path(d));
        let input = Input {
            root,
            name,
            url: &url,
            identity: who,
            now: at(secs),
        };
        step(&folder_of(d, who), &input).unwrap()
    }

    fn folder_path_of(d: &Scratch) -> String {
        folder_path(d).display().to_string()
    }

    fn on_folder(d: &Scratch, id: &str, n: u64) -> Option<Vec<u8>> {
        fs::read(folder_path(d).join(transport::manifest_path(id, n))).ok()
    }

    fn local(root: &Path, id: &str) -> Scope {
        manifest::read_scope(root, id).unwrap()
    }

    fn lists(root: &Path, id: &str, who: &Identity) -> bool {
        local(root, id)
            .latest()
            .unwrap()
            .manifest
            .lists(&who.device.id())
    }

    #[test]
    fn a_new_scope_syncs_at_once() {
        let d = scratch("new");
        let root = store(&d, "a");
        let id = make(&root, &a(), "personal", &[]);
        assert_eq!(local(&root, &id).pending.len(), 1);
        let out = go(&d, &root, &a(), "personal", 0);
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
        assert!(out.stop.is_none() && out.events.is_empty() && out.full.is_none());
        let now = local(&root, &id);
        assert!(now.pending.is_empty());
        assert_eq!(
            on_folder(&d, &id, 1).as_deref(),
            Some(&now.versions[0].bytes[..])
        );
        assert!(read_state(&root, &id).published.is_empty());
    }

    #[test]
    fn a_revocation_waits_ten_minutes_on_a_folder() {
        let d = scratch("revoke");
        let root = store(&d, "a");
        let id = make(&root, &a(), "personal", &[&b(), &c()]);
        go(&d, &root, &a(), "personal", 0);
        revoke(&root, &a(), &id, &c());
        let who = a();
        let me = Recipient::device(&who.device);
        let usable = |root: &Path| {
            let scope = local(root, &id);
            let opened = manifest::open(&scope, &me).unwrap().unwrap();
            manifest::usable_epoch(&scope, &opened)
        };
        let out = go(&d, &root, &a(), "personal", 10);
        assert!(out.stop.is_none() && out.events.is_empty());
        assert!(on_folder(&d, &id, 2).is_some());
        assert!(local(&root, &id).pending.contains(&2));
        assert_eq!(usable(&root), Some(1));
        go(&d, &root, &a(), "personal", 10 + 599);
        assert!(local(&root, &id).pending.contains(&2));
        assert_eq!(usable(&root), Some(1));
        let out = go(&d, &root, &a(), "personal", 10 + 600);
        assert!(out.events.is_empty(), "an own version is not reported");
        assert!(local(&root, &id).pending.is_empty());
        assert_eq!(usable(&root), Some(2));
        assert!(read_state(&root, &id).published.is_empty());
        assert!(recent(&root, &id, at(700)).unwrap().is_empty());
    }

    #[test]
    fn a_version_that_keeps_the_epoch_confirms_at_once() {
        let d = scratch("keeps");
        let root = store(&d, "a");
        let id = make(&root, &a(), "personal", &[&b()]);
        go(&d, &root, &a(), "personal", 0);
        add(&root, &a(), &id, &moria());
        let out = go(&d, &root, &a(), "personal", 1);
        assert!(out.events.is_empty());
        assert!(local(&root, &id).pending.is_empty());
        assert!(on_folder(&d, &id, 2).is_some());
    }

    #[test]
    fn a_forged_manifest_is_ignored_and_named_once() {
        let d = scratch("forged");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&b()]);
        go(&d, &ra, &a(), "personal", 0);
        copy_in(&rb, &ra, &id);
        add(&rb, &b(), &id, &moria());
        let mut forged = local(&rb, &id).versions[1].bytes.clone();
        let middle = forged.len() / 2;
        forged[middle] ^= 1;
        fs::write(
            folder_path(&d).join(transport::manifest_path(&id, 2)),
            &forged,
        )
        .unwrap();
        let out = go(&d, &ra, &a(), "personal", 5);
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
        assert_eq!(out.events.len(), 1);
        assert!(out.events[0].starts_with("sync personal: manifest/2.json is invalid"));
        assert_eq!(local(&ra, &id).versions.len(), 1);
        let again = go(&d, &ra, &a(), "personal", 35);
        assert!(again.events.is_empty());
        assert_eq!(local(&ra, &id).versions.len(), 1);
    }

    #[test]
    fn two_writers_of_one_version() {
        let d = scratch("fork");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&b(), &c()]);
        go(&d, &ra, &a(), "personal", 0);
        copy_in(&rb, &ra, &id);
        revoke(&ra, &a(), &id, &c());
        add(&rb, &b(), &id, &moria());
        go(&d, &ra, &a(), "personal", 10);
        let mine = local(&rb, &id).versions[1].bytes.clone();
        assert_ne!(on_folder(&d, &id, 2).unwrap(), mine);

        let waiting = go(&d, &rb, &b(), "personal", 20);
        assert!(waiting.events.is_empty());
        assert_eq!(
            local(&rb, &id).versions.len(),
            2,
            "the winner's epoch change waits"
        );
        let out = go(&d, &rb, &b(), "personal", 20 + 600);
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
        assert!(
            out.events
                .iter()
                .any(|e| e.contains("manifest 2 was written elsewhere first"))
        );
        assert!(
            out.events
                .iter()
                .any(|e| e.contains("epoch changed (manifest 2)"))
        );
        let lost = fs::read(rb.join(format!(".bilbo/scopes/{id}/manifest/lost/2.json"))).unwrap();
        assert_eq!(lost, mine);
        let now = local(&rb, &id);
        assert_eq!(now.versions.len(), 3);
        assert!(
            now.pending.is_empty(),
            "the version written again is published and confirmed"
        );
        assert_eq!(
            on_folder(&d, &id, 3).as_deref(),
            Some(&now.versions[2].bytes[..])
        );
        assert!(lists(&rb, &id, &moria()) && !lists(&rb, &id, &c()));

        let out = go(&d, &ra, &a(), "personal", 700);
        assert!(
            out.events
                .iter()
                .any(|e| e == "sync personal: device moria added by owner key (manifest 3)")
        );
        assert!(local(&ra, &id).pending.is_empty());
        assert!(lists(&ra, &id, &moria()) && !lists(&ra, &id, &c()));
    }

    #[test]
    fn a_confirmed_version_replaced_stops_the_scope() {
        let d = scratch("replaced");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&b()]);
        add(&ra, &a(), &id, &moria());
        go(&d, &ra, &a(), "personal", 0);
        assert!(local(&ra, &id).pending.is_empty());
        let unchanged = go(&d, &ra, &a(), "personal", 1);
        assert!(unchanged.stop.is_none());

        let first = local(&ra, &id).versions[0].bytes.clone();
        let lock = manifest::lock(&rb).unwrap();
        manifest::adopt(&lock, &id, 1, &first).unwrap();
        drop(lock);
        add(&rb, &b(), &id, &c());
        let other = local(&rb, &id).versions[1].bytes.clone();
        fs::write(
            folder_path(&d).join(transport::manifest_path(&id, 2)),
            &other,
        )
        .unwrap();

        let out = go(&d, &ra, &a(), "personal", 3600);
        assert!(out.scope.is_none());
        let stop = out.stop.unwrap();
        assert_eq!(stop.reason, Reason::Differs);
        assert_eq!(
            stop.line,
            "sync personal: manifest 2 on the transport differs from the confirmed one; run bilbo device to compare"
        );
        let again = go(&d, &ra, &a(), "personal", 3601);
        assert_eq!(again.stop.map(|s| s.reason), Some(Reason::Differs));
        assert_eq!(local(&ra, &id).versions.len(), 2);
    }

    #[test]
    fn a_device_added_elsewhere_is_shown_and_nothing_is_sealed() {
        let d = scratch("added");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&b()]);
        go(&d, &ra, &a(), "personal", 0);
        copy_in(&rb, &ra, &id);
        add(&rb, &b(), &id, &moria());
        go(&d, &rb, &b(), "personal", 1);

        let out = go(&d, &ra, &a(), "personal", 2);
        assert_eq!(
            out.events,
            ["sync personal: device moria added by owner key (manifest 2)"]
        );
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
        let now = local(&ra, &id);
        assert_eq!(now.versions.len(), 2);
        assert!(now.pending.is_empty(), "this device wrote no version");
        assert_eq!(
            on_folder(&d, &id, 2).as_deref(),
            Some(&now.versions[1].bytes[..])
        );
        assert!(go(&d, &ra, &a(), "personal", 3).events.is_empty());

        let listed = recent(&ra, &id, at(4)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (
                listed[0].n,
                listed[0].kind,
                listed[0].device.as_deref(),
                listed[0].signer.as_str()
            ),
            (2, Kind::Device, Some("moria"), "owner key")
        );
        let later = 31 * 24 * 3600;
        assert!(recent(&ra, &id, at(later)).unwrap().is_empty());
        go(&d, &ra, &a(), "personal", later);
        let file = fs::read_to_string(changes_path(&ra, &id)).unwrap();
        assert!(file.trim().is_empty(), "a trim drops what is past 30 days");
    }

    #[test]
    fn an_epoch_nobody_here_revoked_for_is_shown_once_the_folder_settled_it() {
        let d = scratch("epoch");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&b(), &c()]);
        go(&d, &ra, &a(), "personal", 0);
        copy_in(&rb, &ra, &id);
        revoke(&rb, &b(), &id, &c());
        go(&d, &rb, &b(), "personal", 1);
        let who = a();
        let me = Recipient::device(&who.device);
        let usable = || {
            let scope = local(&ra, &id);
            let opened = manifest::open(&scope, &me).unwrap().unwrap();
            manifest::usable_epoch(&scope, &opened)
        };
        let waiting = go(&d, &ra, &a(), "personal", 2);
        assert!(waiting.events.is_empty());
        assert_eq!(local(&ra, &id).versions.len(), 1);
        assert_eq!(go(&d, &ra, &a(), "personal", 2 + 599).events.len(), 0);
        assert_eq!(usable(), Some(1));
        let out = go(&d, &ra, &a(), "personal", 2 + 600);
        assert_eq!(out.events, ["sync personal: epoch changed (manifest 2)"]);
        assert_eq!(usable(), Some(2));
        let listed = recent(&ra, &id, at(700)).unwrap();
        assert_eq!((listed[0].n, listed[0].kind), (2, Kind::Epoch));
        assert!(go(&d, &ra, &a(), "personal", 704).events.is_empty());
    }

    #[test]
    fn a_removed_device_stops_and_writes_nothing() {
        let d = scratch("removed");
        let (ra, rc) = (store(&d, "a"), store(&d, "c"));
        let id = make(&ra, &a(), "personal", &[&c()]);
        go(&d, &ra, &a(), "personal", 0);
        copy_in(&rc, &ra, &id);
        revoke(&ra, &a(), &id, &c());
        go(&d, &ra, &a(), "personal", 1);
        let before = on_folder(&d, &id, 2).unwrap();

        let waiting = go(&d, &rc, &c(), "personal", 2);
        assert!(waiting.stop.is_none(), "the new epoch waits on a folder");
        assert_eq!(local(&rc, &id).versions.len(), 1);
        let out = go(&d, &rc, &c(), "personal", 2 + 600);
        assert!(out.scope.is_none());
        let stop = out.stop.unwrap();
        assert_eq!(stop.reason, Reason::Removed);
        assert_eq!(
            stop.line,
            "sync personal: this device was removed from the scope"
        );
        let now = local(&rc, &id);
        assert_eq!(now.versions.len(), 2);
        assert!(now.pending.is_empty());
        assert_eq!(on_folder(&d, &id, 2).unwrap(), before);
        assert!(on_folder(&d, &id, 3).is_none());
    }

    #[test]
    fn a_device_the_scope_does_not_list_syncs_nothing_and_copies_nothing() {
        let d = scratch("none");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[]);
        go(&d, &ra, &a(), "personal", 0);
        let out = go(&d, &rb, &b(), "personal", 1);
        assert!(out.scope.is_none() && out.events.is_empty());
        let stop = out.stop.unwrap();
        assert_eq!(stop.reason, Reason::NotInScope);
        assert_eq!(
            stop.line,
            "sync personal: this device is not in the scope; run bilbo device recover on this device"
        );
        assert!(manifest::scope_ids(&rb).unwrap().is_empty());
        assert!(!rb.join(format!(".bilbo/scopes/{id}")).exists());
    }

    #[test]
    fn a_store_without_the_manifest_restores_the_one_scope_that_lists_it() {
        let d = scratch("restore");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&b(), &c()]);
        go(&d, &ra, &a(), "personal", 0);
        revoke(&ra, &a(), &id, &c());
        go(&d, &ra, &a(), "personal", 1);
        go(&d, &ra, &a(), "personal", 601);
        assert!(local(&ra, &id).pending.is_empty());

        let out = go(&d, &rb, &b(), "personal", 602);
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
        assert!(out.stop.is_none() && out.error.is_none());
        let now = local(&rb, &id);
        assert_eq!(now.versions.len(), 1, "the epoch change waits on a folder");
        assert!(now.pending.is_empty());
        assert_eq!(
            on_folder(&d, &id, 1).as_deref(),
            Some(&now.versions[0].bytes[..])
        );
        let later = go(&d, &rb, &b(), "personal", 602 + 600);
        assert_eq!(later.events, ["sync personal: epoch changed (manifest 2)"]);
        assert_eq!(local(&rb, &id).versions.len(), 2);
    }

    #[test]
    fn several_scopes_of_the_name_that_list_the_device_restore_none() {
        let d = scratch("several");
        let (ra, rb, rc) = (store(&d, "a"), store(&d, "b"), store(&d, "c"));
        let first = make(&ra, &a(), "personal", &[&b()]);
        put(&d, &ra, &a(), &first);
        let second = make(&rc, &c(), "personal", &[&b()]);
        put(&d, &rc, &c(), &second);
        let out = go(&d, &rb, &b(), "personal", 0);
        assert!(out.scope.is_none());
        let stop = out.stop.unwrap();
        assert_eq!(stop.reason, Reason::NotInScope);
        assert_eq!(
            stop.line,
            "sync personal: the folder holds several scopes named personal; run bilbo device recover on this device"
        );
        assert!(manifest::scope_ids(&rb).unwrap().is_empty());
    }

    #[test]
    fn a_sibling_scope_in_the_folder_is_neither_adopted_nor_preferred() {
        let d = scratch("sibling");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let own = make(&rb, &b(), "personal", &[]);
        go(&d, &rb, &b(), "personal", 0);
        let sibling = make(&ra, &a(), "personal", &[&b(), &c(), &moria()]);
        put(&d, &ra, &a(), &sibling);
        assert!(on_folder(&d, &sibling, 1).is_some());

        let out = go(&d, &rb, &b(), "personal", 2);
        assert_eq!(out.scope.as_deref(), Some(own.as_str()));
        assert!(out.events.is_empty() && out.replaced.is_empty());
        assert_eq!(
            manifest::scope_ids(&rb).unwrap(),
            std::slice::from_ref(&own)
        );
        assert_eq!(local(&rb, &own).versions.len(), 1);
    }

    #[test]
    fn no_version_1_goes_beside_a_scope_the_device_cannot_open() {
        let d = scratch("outsider");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let theirs = make(&ra, &a(), "personal", &[]);
        go(&d, &ra, &a(), "personal", 0);
        let minted = make(&rb, &b(), "personal", &[]);
        let out = go(&d, &rb, &b(), "personal", 1);
        assert!(out.scope.is_none());
        assert_eq!(out.stop.unwrap().reason, Reason::NotInScope);
        assert!(on_folder(&d, &minted, 1).is_none());
        assert!(local(&rb, &minted).pending.contains(&1));
        assert!(on_folder(&d, &theirs, 1).is_some());
    }

    #[test]
    fn a_member_creates_a_scope_beside_one_it_cannot_open() {
        let d = scratch("member");
        let (ra, rc) = (store(&d, "a"), store(&d, "c"));
        let personal = make(&ra, &a(), "personal", &[&c()]);
        make(&ra, &a(), "shared", &[]);
        go(&d, &ra, &a(), "personal", 0);
        go(&d, &ra, &a(), "shared", 0);
        copy_in(&rc, &ra, &personal);
        let notes2 = make(&rc, &c(), "notes2", &[]);
        let out = go(&d, &rc, &c(), "notes2", 1);
        assert_eq!(out.scope.as_deref(), Some(notes2.as_str()));
        assert!(on_folder(&d, &notes2, 1).is_some());
        assert!(local(&rc, &notes2).pending.is_empty());
    }

    #[test]
    fn a_folder_without_this_devices_scopes() {
        let d = scratch("empty");
        let root = store(&d, "a");
        let id = make(&root, &a(), "personal", &[]);
        let in_step = go(&d, &root, &a(), "personal", 0);
        assert!(in_step.stop.is_none());
        assert!(go(&d, &root, &a(), "personal", 1).stop.is_none());

        let moved = d.0.join("moved");
        fs::create_dir_all(&moved).unwrap();
        let url = url_of(&moved);
        let input = Input {
            root: &root,
            name: "personal",
            url: &url,
            identity: &a(),
            now: at(2),
        };
        let out = step(&Folder::new(moved.clone(), &a().device.id()), &input).unwrap();
        assert!(out.scope.is_none());
        let stop = out.stop.unwrap();
        assert_eq!(stop.reason, Reason::NoScopes);
        assert_eq!(
            stop.line,
            format!(
                "sync personal: {} holds none of this device's scopes; if the folder moved, change scope.personal.sync on every device",
                moved.display()
            )
        );
        assert_eq!(
            fs::read_dir(&moved).unwrap().count(),
            0,
            "nothing is pushed there"
        );
        assert!(local(&root, &id).pending.is_empty());
    }

    #[test]
    fn the_pin_stops_a_scope_before_the_transport_is_touched() {
        let d = scratch("pin");
        let root = store(&d, "a");
        make_pinned(&root, &a(), "personal", "https://relay.example", &[]);
        let missing = d.0.join("missing");
        let url = url_of(&missing);
        let input = Input {
            root: &root,
            name: "personal",
            url: &url,
            identity: &a(),
            now: at(0),
        };
        let out = step(&Folder::new(missing, &a().device.id()), &input).unwrap();
        let stop = out.stop.unwrap();
        assert_eq!(stop.reason, Reason::Pin);
        assert_eq!(
            stop.line,
            format!(
                "sync personal: the manifest pins https://relay.example, the config says {url}; run bilbo device init to move the scope, or set the config back"
            )
        );
    }

    #[test]
    fn another_folder_path_under_a_file_pin_is_no_stop() {
        let d = scratch("path");
        let root = store(&d, "a");
        make(&root, &a(), "personal", &[]);
        assert!(go(&d, &root, &a(), "personal", 0).stop.is_none());
    }

    #[test]
    fn a_folder_that_is_not_there_is_an_error() {
        let d = scratch("gone");
        let root = store(&d, "a");
        make(&root, &a(), "personal", &[]);
        let missing = d.0.join("missing");
        let url = url_of(&missing);
        let input = Input {
            root: &root,
            name: "personal",
            url: &url,
            identity: &a(),
            now: at(0),
        };
        let out = step(&Folder::new(missing.clone(), &a().device.id()), &input).unwrap();
        assert_eq!(out.error.as_deref(), Some("the folder does not exist"));
        assert!(out.scope.is_none() && out.stop.is_none());
        assert!(!missing.exists());
    }

    /// A folder whose creates are refused as full.
    struct Full(Folder);

    impl Transport for Full {
        fn reachable(&self) -> Result<(), String> {
            self.0.reachable()
        }
        fn keeps(&self) -> bool {
            self.0.keeps()
        }
        fn scopes(&self) -> Result<Vec<String>, String> {
            self.0.scopes()
        }
        fn devices(&self, scope: &str) -> Result<Vec<String>, String> {
            self.0.devices(scope)
        }
        fn list_after(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
            self.0.list_after(scope, device, cursor)
        }
        fn probe(&self, scope: &str, device: &str, cursor: u64) -> Result<Vec<u64>, String> {
            self.0.probe(scope, device, cursor)
        }
        fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
            self.0.get(path)
        }
        fn create(&self, _path: &str, _bytes: &[u8]) -> Put {
            Put::Full("No space left on device".into())
        }
        fn highest_manifest(&self, scope: &str) -> Result<Option<u64>, String> {
            self.0.highest_manifest(scope)
        }
        fn replace(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
            self.0.replace(path, bytes)
        }
        fn sweep(&self, now: SystemTime) -> Result<(), String> {
            self.0.sweep(now)
        }
        fn remove_mailbox(&self, nameplate: &str) -> Result<(), String> {
            self.0.remove_mailbox(nameplate)
        }
    }

    #[test]
    fn a_full_transport_leaves_the_version_pending_and_says_why() {
        let d = scratch("full");
        let root = store(&d, "a");
        let id = make(&root, &a(), "personal", &[]);
        let url = url_of(&folder_path(&d));
        let input = Input {
            root: &root,
            name: "personal",
            url: &url,
            identity: &a(),
            now: at(0),
        };
        let out = step(&Full(folder_of(&d, &a())), &input).unwrap();
        assert_eq!(out.full.as_deref(), Some("No space left on device"));
        assert!(out.stop.is_none());
        assert!(local(&root, &id).pending.contains(&1));
        assert!(on_folder(&d, &id, 1).is_none());
        let again = go(&d, &root, &a(), "personal", 1);
        assert!(again.full.is_none());
        assert!(local(&root, &id).pending.is_empty());
    }

    #[test]
    fn two_published_scopes_with_one_name_keep_the_one_listing_more_devices() {
        let d = scratch("names");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let own = make(&rb, &b(), "personal", &[]);
        go(&d, &rb, &b(), "personal", 0);
        let theirs = make(&ra, &a(), "personal", &[]);
        add(&ra, &a(), &theirs, &b());
        put(&d, &ra, &a(), &theirs);
        copy_in(&rb, &ra, &theirs);
        assert!(local(&rb, &own).pending.is_empty(), "both were published");

        let out = go(&d, &rb, &b(), "personal", 2);
        assert_eq!(out.scope.as_deref(), Some(theirs.as_str()));
        assert_eq!(out.replaced, std::slice::from_ref(&own));
        assert_eq!(
            out.events,
            [format!("sync personal: scope {theirs} replaces {own}")]
        );
        let again = go(&d, &rb, &b(), "personal", 3);
        assert_eq!(again.scope.as_deref(), Some(theirs.as_str()));
        assert!(again.events.is_empty(), "the line is said once");
    }

    #[test]
    fn a_scope_the_device_cannot_open_makes_no_replaces_line() {
        let d = scratch("closed");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let own = make(&rb, &b(), "personal", &[]);
        go(&d, &rb, &b(), "personal", 0);
        let theirs = make(&ra, &a(), "personal", &[]);
        put(&d, &ra, &a(), &theirs);
        let out = go(&d, &rb, &b(), "personal", 1);
        assert_eq!(out.scope.as_deref(), Some(own.as_str()));
        assert!(out.events.is_empty() && out.replaced.is_empty());
    }

    /// Version 3 at epoch 3, signed with the owner seed by a device that never held epoch 2: its chain entry for
    /// epoch 2 decrypts to a wrong key, so a member that holds epoch 2 must reject it. It passes every secret-free
    /// check.
    fn forge_rotation(root: &Path, id: &str, thief: &Identity) -> Vec<u8> {
        let scope = local(root, id);
        let v2 = &scope.versions[1];
        let mut m = v2.manifest.clone();
        let (k3, wrong) = (
            keys::random_secret().unwrap(),
            keys::random_secret().unwrap(),
        );
        m.n = 3;
        m.prev = Some(hash::sha256_hex(&v2.bytes));
        m.epoch = 3;
        m.chain.push(Link {
            epoch: 2,
            key: keys::hex(&keys::encrypt(&k3, &keys::chain_aad(id, 2), &wrong[..]).unwrap()),
        });
        let members: Vec<Member> = m.devices.iter().filter_map(Member::from_entry).collect();
        m.sealed = manifest::seal_all(id, 3, &k3, &members, &thief.owner.box_public).unwrap();
        m.name = manifest::seal_name(&k3, &m, "personal").unwrap();
        manifest::signed(m, &thief.owner.sign).1
    }

    /// A scope of A, B and C whose version 2 (C revoked, epoch 2) both A and B hold confirmed.
    fn rotated(d: &Scratch) -> (PathBuf, PathBuf, String) {
        let (ra, rb) = (store(d, "a"), store(d, "b"));
        let id = make(&ra, &a(), "personal", &[&b(), &c()]);
        go(d, &ra, &a(), "personal", 0);
        copy_in(&rb, &ra, &id);
        revoke(&ra, &a(), &id, &c());
        go(d, &ra, &a(), "personal", 10);
        go(d, &ra, &a(), "personal", 610);
        assert!(local(&ra, &id).pending.is_empty());
        go(d, &rb, &b(), "personal", 611);
        go(d, &rb, &b(), "personal", 1211);
        assert_eq!(local(&rb, &id).versions.len(), 2);
        (ra, rb, id)
    }

    #[test]
    fn a_rotation_the_member_chain_check_rejects_is_not_adopted_through_a_fork() {
        let d = scratch("forged-rotation");
        let (ra, rb, id) = rotated(&d);
        add(&rb, &b(), &id, &moria());
        assert!(local(&rb, &id).pending.contains(&3));
        let forged = forge_rotation(&ra, &id, &c());
        fs::write(
            folder_path(&d).join(transport::manifest_path(&id, 3)),
            &forged,
        )
        .unwrap();
        let chain = scopes::chain(&folder_of(&d, &b()), &id).unwrap();
        assert!(chain.invalid.is_none(), "valid without a secret");
        assert!(manifest::read(&chain, &Recipient::device(&b().device)).is_err());

        let out = go(&d, &rb, &b(), "personal", 1300);
        assert!(out.error.is_none() && out.stop.is_none());
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
        assert_eq!(out.events.len(), 1);
        assert!(out.events[0].starts_with("sync personal: manifest/3.json is invalid"));
        let now = local(&rb, &id);
        assert_eq!((now.versions.len(), now.pending.contains(&3)), (3, true));
        assert_ne!(now.versions[2].bytes, forged);
        assert!(
            !rb.join(format!(".bilbo/scopes/{id}/manifest/lost"))
                .exists()
        );
        assert!(go(&d, &rb, &b(), "personal", 1400).events.is_empty());
        let past = go(&d, &rb, &b(), "personal", 1300 + 600);
        assert!(past.error.is_none() && past.events.is_empty());
        let now = local(&rb, &id);
        assert_eq!((now.versions.len(), now.pending.contains(&3)), (3, true));
        assert_ne!(now.versions[2].bytes, forged);
        assert!(
            !rb.join(format!(".bilbo/scopes/{id}/manifest/lost"))
                .exists()
        );
    }

    #[test]
    fn a_rotation_the_member_chain_check_rejects_is_not_adopted() {
        let d = scratch("forged-adopt");
        let (ra, rb, id) = rotated(&d);
        let forged = forge_rotation(&ra, &id, &c());
        fs::write(
            folder_path(&d).join(transport::manifest_path(&id, 3)),
            &forged,
        )
        .unwrap();
        let out = go(&d, &rb, &b(), "personal", 1300);
        assert!(out.error.is_none() && out.stop.is_none());
        assert_eq!(out.events.len(), 1);
        assert!(out.events[0].contains("manifest/3.json is invalid"));
        assert_eq!(local(&rb, &id).versions.len(), 2);
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn nothing_is_adopted_above_an_own_unsettled_version() {
        let d = scratch("cap");
        let (rb, rc) = (store(&d, "b"), store(&d, "c"));
        let id = make(&rb, &b(), "personal", &[&c(), &moria()]);
        go(&d, &rb, &b(), "personal", 0);
        copy_in(&rc, &rb, &id);
        revoke(&rb, &b(), &id, &moria());
        go(&d, &rb, &b(), "personal", 1);
        let mine = local(&rb, &id).versions[1].bytes.clone();
        let lock = manifest::lock(&rc).unwrap();
        manifest::adopt(&lock, &id, 2, &mine).unwrap();
        drop(lock);
        add(&rc, &c(), &id, &identity(0, "gimli", 9));
        let above = local(&rc, &id).versions[2].bytes.clone();
        let t = folder_of(&d, &c());
        assert_eq!(
            t.create(&transport::manifest_path(&id, 3), &above),
            Put::Created
        );

        let out = go(&d, &rb, &b(), "personal", 2);
        assert!(out.error.is_none() && out.events.is_empty());
        let now = local(&rb, &id);
        assert_eq!((now.versions.len(), now.pending.contains(&2)), (2, true));
    }

    #[test]
    fn no_version_1_goes_beside_a_scope_still_arriving_or_one_of_the_name_that_lists_it() {
        let d = scratch("arriving");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let theirs = make(&ra, &a(), "personal", &[&b()]);
        add(&ra, &a(), &theirs, &moria());
        let t = folder_of(&d, &a());
        let second = local(&ra, &theirs).versions[1].bytes.clone();
        assert_eq!(
            t.create(&transport::manifest_path(&theirs, 2), &second),
            Put::Created
        );
        let minted = make(&rb, &b(), "personal", &[]);
        let out = go(&d, &rb, &b(), "personal", 0);
        assert!(out.scope.is_none());
        let stop = out.stop.unwrap();
        assert_eq!(stop.reason, Reason::NotInScope);
        assert!(stop.line.starts_with(&format!(
            "sync personal: {} holds scope {theirs} that does not verify: ",
            folder_path_of(&d)
        )));
        assert!(on_folder(&d, &minted, 1).is_none());

        let first = local(&ra, &theirs).versions[0].bytes.clone();
        assert_eq!(
            t.create(&transport::manifest_path(&theirs, 1), &first),
            Put::Created
        );
        let out = go(&d, &rb, &b(), "personal", 1);
        assert_eq!(out.stop.unwrap().reason, Reason::NotInScope);
        assert!(on_folder(&d, &minted, 1).is_none());
        assert!(local(&rb, &minted).pending.contains(&1));
    }

    #[test]
    fn a_confirmed_version_the_folder_lost_is_written_back() {
        let d = scratch("writeback");
        let root = store(&d, "a");
        let id = make(&root, &a(), "personal", &[&b()]);
        go(&d, &root, &a(), "personal", 0);
        add(&root, &a(), &id, &moria());
        go(&d, &root, &a(), "personal", 1);
        add(&root, &a(), &id, &c());
        go(&d, &root, &a(), "personal", 2);
        assert_eq!(local(&root, &id).versions.len(), 3);
        let path = folder_path(&d).join(transport::manifest_path(&id, 2));
        let bytes = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let out = go(&d, &root, &a(), "personal", 3);
        assert!(out.stop.is_none() && out.events.is_empty() && out.error.is_none());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn a_folder_that_lost_everything_is_not_written_back() {
        let d = scratch("nowriteback");
        let root = store(&d, "a");
        let id = make(&root, &a(), "personal", &[]);
        go(&d, &root, &a(), "personal", 0);
        fs::remove_dir_all(folder_path(&d).join("scopes")).unwrap();
        let out = go(&d, &root, &a(), "personal", 1);
        assert_eq!(out.stop.unwrap().reason, Reason::NoScopes);
        assert!(on_folder(&d, &id, 1).is_none());
    }

    /// A thief holding the owner seed makes its own `personal` listing B and publishes it.
    fn thief_scope(d: &Scratch, target: &Identity) -> String {
        let rt = store(d, "thief");
        let id = make(&rt, &c(), "personal", &[target]);
        put(d, &rt, &c(), &id);
        id
    }

    const SEVERAL: &str = "sync personal: the folder holds several scopes named personal; run bilbo device recover on this device";

    #[test]
    fn a_real_scope_with_trailing_garbage_is_still_a_candidate_beside_a_thiefs() {
        let d = scratch("rr-garbage-tail");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let real = make(&ra, &a(), "personal", &[&b()]);
        go(&d, &ra, &a(), "personal", 0);
        thief_scope(&d, &b());
        fs::write(
            folder_path(&d).join(transport::manifest_path(&real, 2)),
            b"not a manifest",
        )
        .unwrap();
        let out = go(&d, &rb, &b(), "personal", 1);
        assert!(out.scope.is_none());
        assert_eq!(out.stop.unwrap().line, SEVERAL);
        assert!(manifest::scope_ids(&rb).unwrap().is_empty());
    }

    #[test]
    fn a_real_scope_with_a_forged_rotation_is_still_a_candidate_beside_a_thiefs() {
        let d = scratch("rr-forged-tail");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let real = make(&ra, &a(), "personal", &[&b(), &c()]);
        go(&d, &ra, &a(), "personal", 0);
        revoke(&ra, &a(), &real, &c());
        go(&d, &ra, &a(), "personal", 10);
        go(&d, &ra, &a(), "personal", 610);
        let forged = forge_rotation(&ra, &real, &c());
        fs::write(
            folder_path(&d).join(transport::manifest_path(&real, 3)),
            &forged,
        )
        .unwrap();
        thief_scope(&d, &b());
        let out = go(&d, &rb, &b(), "personal", 611);
        assert_eq!(out.stop.unwrap().line, SEVERAL);
        assert!(manifest::scope_ids(&rb).unwrap().is_empty());
    }

    #[test]
    fn a_scope_without_its_version_1_blocks_the_restore() {
        let d = scratch("rr-no-v1");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let real = make(&ra, &a(), "personal", &[&b()]);
        add(&ra, &a(), &real, &moria());
        go(&d, &ra, &a(), "personal", 0);
        thief_scope(&d, &b());
        fs::remove_file(folder_path(&d).join(transport::manifest_path(&real, 1))).unwrap();
        let out = go(&d, &rb, &b(), "personal", 1);
        assert!(out.scope.is_none());
        let stop = out.stop.unwrap();
        assert!(stop.line.starts_with(&format!(
            "sync personal: {} holds scope {real} that does not verify: ",
            folder_path_of(&d)
        )));
        assert!(manifest::scope_ids(&rb).unwrap().is_empty());
    }

    #[test]
    fn an_honest_scope_with_trailing_garbage_restores_its_prefix() {
        let d = scratch("rr-honest-garbage");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let real = make(&ra, &a(), "personal", &[&b()]);
        go(&d, &ra, &a(), "personal", 0);
        fs::write(
            folder_path(&d).join(transport::manifest_path(&real, 2)),
            b"not a manifest",
        )
        .unwrap();
        let out = go(&d, &rb, &b(), "personal", 1);
        assert_eq!(out.scope.as_deref(), Some(real.as_str()));
        assert!(out.error.is_none() && out.stop.is_none());
        assert!(out.events[0].starts_with("sync personal: resumed scope"));
        let invalid: Vec<_> = out
            .events
            .iter()
            .filter(|e| e.contains("manifest/2.json is invalid"))
            .collect();
        assert_eq!(invalid.len(), 1);
        assert_eq!(local(&rb, &real).versions.len(), 1);
        assert!(go(&d, &rb, &b(), "personal", 2).events.is_empty());
    }

    #[test]
    fn a_device_listed_after_a_rotation_is_resumed_once_the_epoch_settled() {
        let d = scratch("rr-after-rotation");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&c()]);
        go(&d, &ra, &a(), "personal", 0);
        revoke(&ra, &a(), &id, &c());
        go(&d, &ra, &a(), "personal", 10);
        go(&d, &ra, &a(), "personal", 610);
        add(&ra, &a(), &id, &b());
        go(&d, &ra, &a(), "personal", 611);
        assert_eq!(local(&ra, &id).versions.len(), 3);

        let waiting = go(&d, &rb, &b(), "personal", 612);
        assert!(waiting.scope.is_none());
        let stop = waiting.stop.unwrap();
        assert_eq!(stop.reason, Reason::Settling);
        assert_eq!(
            stop.line,
            format!("sync personal: waiting for the folder to settle scope {id}")
        );
        assert!(local_versions(&rb, &id).is_empty());
        let still = go(&d, &rb, &b(), "personal", 612 + 599);
        assert_eq!(still.stop.unwrap().reason, Reason::Settling);
        let out = go(&d, &rb, &b(), "personal", 612 + 600);
        assert_eq!(out.scope.as_deref(), Some(id.as_str()));
        assert!(out.stop.is_none() && out.error.is_none());
        assert_eq!(
            out.events[0],
            format!("sync personal: resumed scope {id} from the folder (2 devices)")
        );
        assert_eq!(local(&rb, &id).versions.len(), 3);
    }

    fn local_versions(root: &Path, id: &str) -> Vec<u64> {
        manifest::read_scope(root, id)
            .map(|s| s.versions.iter().map(|v| v.manifest.n).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_damaged_local_scope_is_an_error_not_a_failed_step() {
        let d = scratch("rr-damaged-local");
        let (ra, rb) = (store(&d, "a"), store(&d, "b"));
        let id = make(&ra, &a(), "personal", &[&b()]);
        go(&d, &ra, &a(), "personal", 0);
        copy_in(&rb, &ra, &id);
        let path = rb.join(format!(".bilbo/scopes/{id}/manifest/1.json"));
        let mut bytes = fs::read(&path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 1;
        fs::write(&path, bytes).unwrap();
        let out = go(&d, &rb, &b(), "personal", 1);
        assert!(out.scope.is_none());
        assert!(out.error.unwrap().contains("manifest/1.json is invalid"));
    }

    #[test]
    fn a_folder_that_is_not_there_is_an_error_without_a_local_manifest_too() {
        let d = scratch("rr-gone-restore");
        let rb = store(&d, "b");
        let missing = d.0.join("missing");
        let url = url_of(&missing);
        let input = Input {
            root: &rb,
            name: "personal",
            url: &url,
            identity: &b(),
            now: at(0),
        };
        let out = step(&Folder::new(missing, &b().device.id()), &input).unwrap();
        assert_eq!(out.error.as_deref(), Some("the folder does not exist"));
        assert!(out.scope.is_none() && out.stop.is_none());
    }
}
