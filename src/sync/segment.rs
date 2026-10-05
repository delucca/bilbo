//! The segment: its envelope, sealing and signing, the plaintext format and the checks on read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::identity::keys::{self, SignKey};
use crate::identity::manifest::{self, Opened};
use crate::note::versions::{self, Declaration, Version};
use crate::shared::{frontmatter, hash};

/// The envelope and plaintext format this code writes and reads.
pub const FORMAT: u32 = 1;
/// The largest segment file.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// The first line of the associated data.
const AAD_FORMAT: &str = "bilbo-segment-1";
/// What the envelope's other keys take, with room to spare.
const ENVELOPE_SLACK: usize = 1024;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// A segment file: padded standard base64 in `nonce`, `ciphertext` and `sig`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: u32,
    scope: String,
    device: String,
    seq: u64,
    epoch: u64,
    nonce: String,
    ciphertext: String,
    sig: String,
}

/// What a segment holds, once decrypted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plaintext {
    pub format: u32,
    /// The writer's time, shown and never used to decide staleness.
    pub at: String,
    #[serde(default)]
    pub records: Vec<Record>,
    #[serde(default)]
    pub blobs: Vec<Blob>,
    /// The highest seq applied from each other device.
    #[serde(default)]
    pub acks: BTreeMap<String, u64>,
    #[serde(default)]
    pub declarations: Vec<Declaration>,
}

/// A plaintext as read: records stay raw, so one that does not parse is skipped and not the segment.
#[derive(Deserialize)]
struct Wire {
    format: u32,
    at: String,
    #[serde(default)]
    records: Vec<Value>,
    #[serde(default)]
    blobs: Vec<Blob>,
    #[serde(default)]
    acks: BTreeMap<String, u64>,
    #[serde(default)]
    declarations: Vec<Declaration>,
}

impl Plaintext {
    pub fn new(at: &str) -> Plaintext {
        Plaintext {
            format: FORMAT,
            at: at.to_string(),
            records: Vec::new(),
            blobs: Vec::new(),
            acks: BTreeMap::new(),
            declarations: Vec::new(),
        }
    }
}

/// A version line and the note it belongs to. A `left` record carries no `file` or `blob` on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct Record {
    pub note: String,
    pub version: Version,
}

impl Record {
    fn is_left(&self) -> bool {
        self.version.event == versions::LEFT
    }
}

impl From<Record> for Value {
    fn from(record: Record) -> Value {
        let Ok(Value::Object(mut map)) = serde_json::to_value(&record.version) else {
            unreachable!("a version serializes to an object")
        };
        if record.is_left() {
            map.remove("file");
            map.remove("blob");
        }
        map.insert("note".into(), Value::String(record.note));
        Value::Object(map)
    }
}

impl TryFrom<Value> for Record {
    type Error = String;

    fn try_from(value: Value) -> Result<Record, String> {
        let Value::Object(mut map) = value else {
            return Err("a record is not an object".into());
        };
        let Some(Value::String(note)) = map.remove("note") else {
            return Err("a record has no note".into());
        };
        if map.get("event").and_then(Value::as_str) == Some(versions::LEFT) {
            map.insert("file".into(), Value::String(String::new()));
            map.insert("blob".into(), Value::String(versions::DELETED.into()));
        }
        let version = serde_json::from_value(Value::Object(map)).map_err(|e| e.to_string())?;
        Ok(Record { note, version })
    }
}

/// A note's bytes under their SHA-256: `text` when they are UTF-8, else `data` in base64.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

impl Blob {
    pub fn new(bytes: &[u8]) -> Blob {
        let (text, data) = match std::str::from_utf8(bytes) {
            Ok(text) => (Some(text.to_string()), None),
            Err(_) => (None, Some(STANDARD.encode(bytes))),
        };
        Blob {
            hash: hash::sha256_hex(bytes),
            text,
            data,
        }
    }

    /// The bytes, when exactly one of `text` and `data` is set and they match `hash`.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        let bytes = match (&self.text, &self.data) {
            (Some(text), None) => text.as_bytes().to_vec(),
            (None, Some(data)) => STANDARD
                .decode(data)
                .map_err(|_| "its data is not base64".to_string())?,
            _ => return Err("it holds neither or both of text and data".into()),
        };
        if hash::sha256_hex(&bytes) != self.hash {
            return Err("its bytes do not match its hash".into());
        }
        Ok(bytes)
    }
}

/// A record that failed a check on read, which the reader reports and does not apply.
#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    pub note: String,
    pub version: String,
    pub why: String,
}

/// A segment that verified: its plaintext without the records and blobs that failed, and those records.
#[derive(Debug)]
pub struct Read {
    pub plaintext: Plaintext,
    pub skipped: Vec<Skipped>,
}

/// Why a segment is not read.
#[derive(Debug, PartialEq)]
pub enum Refusal {
    /// A `format` this bilbo does not read.
    Upgrade(u32),
    Failed(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Upgrade(format) => write!(
                f,
                "it has format {format}, which this bilbo does not read; upgrade bilbo"
            ),
            Refusal::Failed(why) => f.write_str(why),
        }
    }
}

fn failed<T>(why: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal::Failed(why.into()))
}

fn aad(scope: &str, device: &str, seq: u64, epoch: u64) -> Vec<u8> {
    format!("{AAD_FORMAT}\n{scope}\n{device}\n{seq}\n{epoch}\n").into_bytes()
}

/// The associated data, then the nonce and the ciphertext, each ending in a newline.
fn signing_input(aad: &[u8], nonce: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let mut input = aad.to_vec();
    input.extend_from_slice(nonce);
    input.push(b'\n');
    input.extend_from_slice(ciphertext);
    input.push(b'\n');
    input
}

/// The file of segment `seq` of `device`'s folder, sealed under `key`, the scope's key at `epoch`.
pub fn seal(
    scope: &str,
    device: &SignKey,
    seq: u64,
    epoch: u64,
    key: &[u8; 32],
    plaintext: &Plaintext,
) -> Result<Vec<u8>, String> {
    let json =
        serde_json::to_vec(plaintext).map_err(|e| format!("cannot encode a segment: {e}"))?;
    seal_json(scope, device, seq, epoch, key, &json)
}

fn seal_json(
    scope: &str,
    device: &SignKey,
    seq: u64,
    epoch: u64,
    key: &[u8; 32],
    json: &[u8],
) -> Result<Vec<u8>, String> {
    let id = keys::device_id(&device.public());
    let aad = aad(scope, &id, seq, epoch);
    let sealed = keys::encrypt(key, &aad, json)?;
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    let sig = device.sign(&signing_input(&aad, nonce, ciphertext));
    let envelope = Envelope {
        format: FORMAT,
        scope: scope.to_string(),
        device: id,
        seq,
        epoch,
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
        sig: STANDARD.encode(sig),
    };
    let bytes =
        serde_json::to_vec(&envelope).map_err(|e| format!("cannot encode a segment: {e}"))?;
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "a segment of {} bytes is over the 8 MiB limit",
            bytes.len()
        ));
    }
    Ok(bytes)
}

/// The signing key of `device` when it may write a segment at `epoch`: some confirmed version at that epoch lists it
/// and the latest confirmed version lists it. A pending version is not confirmed.
fn writer_key(scope: &manifest::Scope, device: &str, epoch: u64) -> Result<[u8; 32], String> {
    let confirmed: Vec<&manifest::Version> = scope
        .versions
        .iter()
        .filter(|v| !scope.pending.contains(&v.manifest.n))
        .collect();
    let latest = confirmed
        .last()
        .ok_or("the scope has no confirmed version")?;
    if !latest.manifest.lists(device) {
        return Err(format!(
            "the latest confirmed version does not list {device}"
        ));
    }
    let entry = confirmed
        .iter()
        .filter(|v| v.manifest.epoch == epoch)
        .find_map(|v| v.manifest.devices.iter().find(|d| d.id == device))
        .ok_or_else(|| format!("no confirmed version at epoch {epoch} lists {device}"))?;
    keys::unhex(&entry.sign).ok_or_else(|| format!("the key of {device} is not valid"))
}

/// The plaintext of the segment `bytes` that `device` wrote as `seq` of `scope`, once every check passes: the
/// format, the envelope, the writer's membership, the signature, the decryption, each blob's hash and each
/// record's id. A failing record or blob is left out and named in `skipped`; anything else is a refusal.
pub fn open(
    bytes: &[u8],
    scope: &str,
    device: &str,
    seq: u64,
    manifests: &manifest::Scope,
    opened: &Opened,
) -> Result<Read, Refusal> {
    if bytes.len() > MAX_BYTES {
        return failed(format!("it is {} bytes, over the 8 MiB limit", bytes.len()));
    }
    let raw: Value = serde_json::from_slice(bytes).or_else(|_| failed("it is not JSON"))?;
    match raw.get("format").map(Value::as_u64) {
        None => return failed("it has no format"),
        Some(Some(n)) if n == u64::from(FORMAT) => {}
        Some(Some(n)) if (2..=u64::from(u32::MAX)).contains(&n) => {
            return Err(Refusal::Upgrade(n as u32));
        }
        Some(_) => return failed("its format is not valid"),
    }
    let envelope: Envelope = serde_json::from_value(raw)
        .or_else(|e| failed(format!("its envelope is not valid: {e}")))?;
    if envelope.scope != scope || envelope.device != device || envelope.seq != seq {
        return failed("it is not the segment its name says");
    }
    if manifests.id != scope {
        return failed("the manifests are another scope's");
    }
    let public = writer_key(manifests, device, envelope.epoch).or_else(failed)?;
    let decode = |text: &str, what: &str| {
        STANDARD
            .decode(text)
            .or_else(|_| failed(format!("its {what} is not base64")))
    };
    let nonce = decode(&envelope.nonce, "nonce")?;
    if nonce.len() != NONCE_LEN {
        return failed("its nonce is not 24 bytes");
    }
    let ciphertext = decode(&envelope.ciphertext, "ciphertext")?;
    let sig: [u8; 64] = decode(&envelope.sig, "sig")?
        .try_into()
        .or_else(|_| failed("its sig is not 64 bytes"))?;
    let aad = aad(scope, device, seq, envelope.epoch);
    if !keys::verify(&public, &signing_input(&aad, &nonce, &ciphertext), &sig) {
        return failed("its signature does not verify");
    }
    let Some(key) = opened.keys.get(&envelope.epoch) else {
        return failed(format!(
            "this device holds no key for epoch {}",
            envelope.epoch
        ));
    };
    let mut sealed = nonce;
    sealed.extend_from_slice(&ciphertext);
    let json = keys::decrypt(key, &aad, &sealed).or_else(failed)?;
    let wire: Wire = serde_json::from_slice(&json)
        .or_else(|e| failed(format!("its plaintext is not valid: {e}")))?;
    if wire.format != FORMAT {
        return failed("its plaintext has another format than its envelope");
    }
    let mut skipped = Vec::new();
    let mut records = Vec::new();
    for value in wire.records {
        let name = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let (note, version) = (name("note"), name("version"));
        match Record::try_from(value) {
            Ok(record) => records.push(record),
            Err(why) => skipped.push(Skipped { note, version, why }),
        }
    }
    let plaintext = Plaintext {
        format: wire.format,
        at: wire.at,
        records,
        blobs: wire.blobs,
        acks: wire.acks,
        declarations: wire.declarations,
    };
    Ok(check(plaintext, skipped))
}

/// Drops the blobs whose hash fails and the records whose fields, id or blob do, after those that did not parse.
fn check(mut plaintext: Plaintext, mut skipped: Vec<Skipped>) -> Read {
    let mut bad: BTreeMap<String, String> = BTreeMap::new();
    plaintext.blobs.retain(|blob| match blob.bytes() {
        Ok(_) => true,
        Err(why) => {
            bad.insert(blob.hash.clone(), format!("blob {}: {why}", blob.hash));
            false
        }
    });
    plaintext.records.retain(|record| {
        let v = &record.version;
        let why = if !frontmatter::is_ulid(&record.note) {
            Some("its note is not a note id".to_string())
        } else if !versions::readable(v) {
            Some("its version, parents, blob or file is not valid".to_string())
        } else if let Some(why) = bad.get(&v.blob) {
            Some(why.clone())
        } else if !record.is_left()
            && versions::version_id(&record.note, &v.parents, &v.file, &v.blob) != v.version
        {
            Some("its id does not match its parents, file and blob".to_string())
        } else {
            None
        };
        if let Some(why) = &why {
            skipped.push(Skipped {
                note: record.note.clone(),
                version: v.version.clone(),
                why: why.clone(),
            });
        }
        why.is_none()
    });
    Read { plaintext, skipped }
}

/// Splits `plaintext` into plaintexts that each seal to at most 8 MiB, by records and never inside a blob. Every
/// part carries the acks, the first the declarations, and each record's blob goes with it.
pub fn split(plaintext: Plaintext) -> Vec<Plaintext> {
    split_within(plaintext, MAX_BYTES)
}

/// The most JSON a segment file of `limit` bytes holds: the base64 of the nonce, the tag and the JSON, and the
/// envelope.
fn json_budget(limit: usize) -> usize {
    (limit.saturating_sub(ENVELOPE_SLACK) / 4 * 3).saturating_sub(NONCE_LEN + TAG_LEN)
}

fn json_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(0, |v| v.len())
}

fn split_within(plaintext: Plaintext, limit: usize) -> Vec<Plaintext> {
    let budget = json_budget(limit);
    let Plaintext {
        at,
        records,
        blobs,
        acks,
        declarations,
        ..
    } = plaintext;
    let blobs: BTreeMap<String, Blob> = blobs.into_iter().map(|b| (b.hash.clone(), b)).collect();
    let part = |declarations: Vec<Declaration>| Plaintext {
        acks: acks.clone(),
        declarations,
        ..Plaintext::new(&at)
    };
    let mut parts = Vec::new();
    let mut current = part(declarations);
    let mut size = json_len(&current);
    let mut held: BTreeSet<String> = BTreeSet::new();
    for record in records {
        let blob = blobs.get(&record.version.blob);
        let cost = json_len(&record) + 1;
        let extra = blob
            .filter(|b| !held.contains(&b.hash))
            .map_or(0, |b| json_len(b) + 1);
        if !current.records.is_empty() && size + cost + extra > budget {
            parts.push(std::mem::replace(&mut current, part(Vec::new())));
            size = json_len(&current);
            held.clear();
        }
        if let Some(blob) = blobs.get(&record.version.blob)
            && held.insert(blob.hash.clone())
        {
            size += json_len(blob) + 1;
            current.blobs.push(blob.clone());
        }
        size += cost;
        current.records.push(record);
    }
    let loose: Vec<Blob> = blobs
        .into_values()
        .filter(|b| !parts_hold(&parts, &current, b))
        .collect();
    for blob in loose {
        let cost = json_len(&blob) + 1;
        if size + cost > budget && (!current.records.is_empty() || !current.blobs.is_empty()) {
            parts.push(std::mem::replace(&mut current, part(Vec::new())));
            size = json_len(&current);
        }
        size += cost;
        current.blobs.push(blob);
    }
    parts.push(current);
    parts
}

fn parts_hold(parts: &[Plaintext], current: &Plaintext, blob: &Blob) -> bool {
    parts
        .iter()
        .chain(std::iter::once(current))
        .any(|p| p.blobs.iter().any(|b| b.hash == blob.hash))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::Map;

    use super::*;
    use crate::identity::keys::{Device, Identity, Owner};
    use crate::identity::manifest::{Member, Recipient};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-segment-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn identity(name: &str, seed: u8) -> Identity {
        Identity {
            owner: Owner::derive(&[0; 16]).file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    /// Two devices in a scope at epoch 1, version 1 confirmed.
    struct World {
        dir: Scratch,
        a: Identity,
        b: Identity,
        scope: String,
    }

    impl World {
        fn new(name: &str) -> World {
            let dir = scratch(name);
            let (a, b) = (identity("a", 1), identity("b", 3));
            let lock = manifest::lock(&dir.0).unwrap();
            let scope =
                manifest::create(&lock, &a, "personal", "file:///x", &[Member::of(&b.device)])
                    .unwrap()
                    .scope;
            drop(lock);
            let world = World { dir, a, b, scope };
            world.confirm_all();
            world
        }

        fn read(&self) -> manifest::Scope {
            manifest::read_scope(&self.dir.0, &self.scope).unwrap()
        }

        fn confirm_all(&self) {
            let lock = manifest::lock(&self.dir.0).unwrap();
            for v in &self.read().versions {
                manifest::confirm(&lock, &self.scope, v.manifest.n, &v.bytes).unwrap();
            }
        }

        fn opened(&self, who: &Identity) -> Opened {
            manifest::open(&self.read(), &Recipient::device(&who.device))
                .unwrap()
                .unwrap()
        }

        fn key(&self, epoch: u64) -> [u8; 32] {
            *self.opened(&self.a).keys[&epoch]
        }

        fn revoke_b(&self) {
            let lock = manifest::lock(&self.dir.0).unwrap();
            manifest::revoke(
                &lock,
                &self.read(),
                &self.opened(&self.a),
                &self.a,
                &self.b.device.id(),
            )
            .unwrap();
            drop(lock);
            self.confirm_all();
        }

        fn seal(&self, who: &Identity, seq: u64, epoch: u64, plaintext: &Plaintext) -> Vec<u8> {
            seal(
                &self.scope,
                &who.device.sign,
                seq,
                epoch,
                &self.key(epoch),
                plaintext,
            )
            .unwrap()
        }

        fn open(&self, bytes: &[u8], who: &Identity, seq: u64) -> Result<Read, Refusal> {
            let id = who.device.id();
            open(
                bytes,
                &self.scope,
                &id,
                seq,
                &self.read(),
                &self.opened(&self.a),
            )
        }
    }

    fn ulid(i: usize) -> String {
        format!("{i:026}")
    }

    fn record(note: &str, parents: &[&str], content: &str) -> Record {
        record_in(note, "plan-topic.md", parents, content)
    }

    fn record_in(note: &str, file: &str, parents: &[&str], content: &str) -> Record {
        let blob = hash::sha256_hex(content.as_bytes());
        let parents: Vec<String> = parents.iter().map(|p| p.to_string()).collect();
        let version = versions::version_id(note, &parents, file, &blob);
        Record {
            note: note.into(),
            version: Version {
                version,
                parents,
                file: file.into(),
                blob,
                event: versions::ADDED.into(),
                at: "2026-10-05T10:00:00+00:00".into(),
                device: Some("device".into()),
                ..Version::default()
            },
        }
    }

    fn plaintext() -> Plaintext {
        let mut p = Plaintext::new("2026-10-05T10:00:00+00:00");
        p.records.push(record_in(
            &ulid(1),
            "plan-release-plan.md",
            &[],
            "ship on friday",
        ));
        p.blobs.push(Blob::new(b"ship on friday"));
        p.records.push(record(&ulid(2), &[], "two"));
        p.blobs.push(Blob::new(b"two"));
        p.acks.insert("other".into(), 7);
        p.declarations.push(Declaration {
            declare: "c".repeat(64),
            reason: "y".into(),
            at: "2026-10-05T10:00:00+00:00".into(),
            device: None,
        });
        p
    }

    #[test]
    fn a_segment_round_trips() {
        let w = World::new("round");
        let mut p = plaintext();
        p.blobs.push(Blob::new(&[0xff, 0xfe, 0x00]));
        let left = Record {
            note: ulid(3),
            version: Version {
                version: "c".repeat(64),
                event: versions::LEFT.into(),
                blob: versions::DELETED.into(),
                at: "2026-10-05T10:00:00+00:00".into(),
                ..Version::default()
            },
        };
        p.records.push(left);
        let bytes = w.seal(&w.b, 3, 1, &p);
        let read = w.open(&bytes, &w.b, 3).unwrap();
        assert_eq!(read.plaintext, p);
        assert!(read.skipped.is_empty());
        assert!(read.plaintext.blobs[2].data.is_some() && read.plaintext.blobs[0].text.is_some());
    }

    #[test]
    fn the_envelope_has_exactly_its_keys_and_nothing_readable() {
        let w = World::new("keys");
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        let Value::Object(map) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("not an object")
        };
        let names: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "ciphertext",
                "device",
                "epoch",
                "format",
                "nonce",
                "scope",
                "seq",
                "sig"
            ]
        );
        let text = String::from_utf8_lossy(&bytes);
        for secret in ["release-plan", "ship on friday", "personal"] {
            assert!(!text.contains(secret), "{secret}");
        }
        let mut extra = map.clone();
        extra.insert("more".into(), Value::Null);
        let with_extra = serde_json::to_vec(&extra).unwrap();
        assert!(w.open(&with_extra, &w.a, 1).is_err());
        let mut short = map;
        short.remove("sig");
        assert!(
            w.open(&serde_json::to_vec(&short).unwrap(), &w.a, 1)
                .is_err()
        );
    }

    #[test]
    fn a_changed_byte_in_any_field_is_refused() {
        let w = World::new("tamper");
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        let Value::Object(map) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("not an object")
        };
        for field in [
            "scope",
            "device",
            "seq",
            "epoch",
            "nonce",
            "ciphertext",
            "sig",
        ] {
            let mut changed = map.clone();
            changed[field] = match &map[field] {
                Value::Number(n) => Value::from(n.as_u64().unwrap() + 1),
                Value::String(s) => {
                    let first = if s.starts_with('A') { "B" } else { "A" };
                    Value::String(format!("{first}{}", &s[1..]))
                }
                other => other.clone(),
            };
            let bytes = serde_json::to_vec(&changed).unwrap();
            assert!(w.open(&bytes, &w.a, 1).is_err(), "{field} changed");
        }
        assert!(w.open(&bytes, &w.a, 2).is_err(), "another seq expected");
        assert!(w.open(&bytes, &w.b, 1).is_err(), "another device expected");
        assert!(w.open(&bytes, &w.a, 1).is_ok());
    }

    #[test]
    fn a_device_the_epochs_manifest_does_not_list_is_refused() {
        let w = World::new("epoch");
        w.revoke_b();
        let c = identity("c", 5);
        let lock = manifest::lock(&w.dir.0).unwrap();
        let key = w.key(2);
        manifest::add_device(&lock, &w.read(), &key, &Member::of(&c.device), &w.a).unwrap();
        drop(lock);
        w.confirm_all();
        let p = plaintext();
        let old = w.seal(&c, 1, 1, &p);
        let why = w.open(&old, &c, 1).unwrap_err().to_string();
        assert!(why.contains("epoch 1"), "{why}");
        let new = w.seal(&c, 1, 2, &p);
        assert!(w.open(&new, &c, 1).is_ok());
    }

    #[test]
    fn a_device_the_latest_confirmed_version_does_not_list_is_refused() {
        let w = World::new("latest");
        let p = plaintext();
        let before = w.seal(&w.b, 1, 1, &p);
        assert!(w.open(&before, &w.b, 1).is_ok());
        w.revoke_b();
        let why = w.open(&before, &w.b, 1).unwrap_err().to_string();
        assert!(why.contains("latest confirmed"), "{why}");
        let stranger = identity("s", 9);
        let bytes = w.seal(&stranger, 1, 1, &p);
        assert!(w.open(&bytes, &stranger, 1).is_err());
    }

    #[test]
    fn a_pending_version_does_not_admit_a_device() {
        let w = World::new("pending");
        let c = identity("c", 5);
        let lock = manifest::lock(&w.dir.0).unwrap();
        let key = w.key(1);
        manifest::add_device(&lock, &w.read(), &key, &Member::of(&c.device), &w.a).unwrap();
        drop(lock);
        let bytes = w.seal(&c, 1, 1, &plaintext());
        assert!(w.open(&bytes, &c, 1).is_err());
        w.confirm_all();
        assert!(w.open(&bytes, &c, 1).is_ok());
    }

    #[test]
    fn an_unknown_format_asks_for_an_upgrade() {
        let w = World::new("format");
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        let mut map: Map<String, Value> = serde_json::from_slice(&bytes).unwrap();
        map["format"] = Value::from(2);
        let bytes = serde_json::to_vec(&map).unwrap();
        assert_eq!(w.open(&bytes, &w.a, 1).unwrap_err(), Refusal::Upgrade(2));
    }

    #[test]
    fn a_record_whose_id_does_not_match_is_skipped_alone() {
        let w = World::new("id");
        let mut p = plaintext();
        p.records[0].version.file = "other.md".into();
        let bad = p.records[0].version.version.clone();
        let read = w.open(&w.seal(&w.a, 1, 1, &p), &w.a, 1).unwrap();
        assert_eq!(read.plaintext.records.len(), 1);
        assert_eq!(read.skipped.len(), 1);
        assert_eq!(read.skipped[0].version, bad);
        assert_eq!(read.skipped[0].note, ulid(1));
    }

    #[test]
    fn a_blob_that_does_not_match_its_hash_is_dropped_with_its_records() {
        let w = World::new("blob");
        let mut p = plaintext();
        p.blobs[0].text = Some("changed".into());
        let read = w.open(&w.seal(&w.a, 1, 1, &p), &w.a, 1).unwrap();
        assert_eq!(read.plaintext.blobs.len(), 1);
        assert_eq!(read.plaintext.records.len(), 1);
        assert_eq!(read.skipped[0].note, ulid(1));
    }

    #[test]
    fn a_segment_over_the_limit_is_not_written_or_read() {
        let w = World::new("big");
        let mut p = Plaintext::new("t");
        p.blobs.push(Blob::new("x".repeat(MAX_BYTES).as_bytes()));
        let key = w.key(1);
        assert!(seal(&w.scope, &w.a.device.sign, 1, 1, &key, &p).is_err());
        assert!(w.open(&vec![b' '; MAX_BYTES + 1], &w.a, 1).is_err());
    }

    fn big(count: usize, size: usize) -> Plaintext {
        let mut p = Plaintext::new("t");
        for i in 0..count {
            let content = format!("{i:08}{}", "x".repeat(size));
            p.records.push(record(&ulid(i), &[], &content));
            p.blobs.push(Blob::new(content.as_bytes()));
        }
        p.acks.insert("other".into(), 1);
        p
    }

    #[test]
    fn twenty_mebibytes_split_into_segments_of_at_most_eight() {
        let w = World::new("split");
        let p = big(20, 1024 * 1024);
        let parts = split(p.clone());
        assert!(parts.len() >= 3, "{} parts", parts.len());
        let mut records = 0;
        for (i, part) in parts.iter().enumerate() {
            let bytes = w.seal(&w.a, i as u64 + 1, 1, part);
            assert!(bytes.len() <= MAX_BYTES);
            let read = w.open(&bytes, &w.a, i as u64 + 1).unwrap();
            assert_eq!(read.plaintext.acks, p.acks);
            records += read.plaintext.records.len();
        }
        assert_eq!(records, 20);
    }

    #[test]
    fn a_small_push_is_one_segment() {
        let parts = split(big(1, 4 * 1024));
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].records.len(), 1);
        assert_eq!(split(Plaintext::new("t")).len(), 1);
    }

    #[test]
    fn a_split_keeps_a_shared_blob_with_each_part_and_the_declarations_once() {
        let mut p = Plaintext::new("t");
        for i in 0..4 {
            p.records.push(record(&ulid(i), &[], "same"));
        }
        p.blobs.push(Blob::new(b"same"));
        p.declarations.push(Declaration {
            declare: "c".repeat(64),
            reason: "y".into(),
            at: "t".into(),
            device: None,
        });
        let parts = split_within(p, 2200);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|part| part.blobs.len() == 1));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.declarations.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(
            parts.iter().map(|part| part.records.len()).sum::<usize>(),
            4
        );
    }

    /// An envelope signed by `w.a` over whatever nonce and ciphertext it is given.
    fn forged(w: &World, seq: u64, epoch: u64, nonce: &[u8], ciphertext: &[u8]) -> Vec<u8> {
        let id = w.a.device.id();
        let aad = aad(&w.scope, &id, seq, epoch);
        let sig =
            w.a.device
                .sign
                .sign(&signing_input(&aad, nonce, ciphertext));
        let envelope = Envelope {
            format: FORMAT,
            scope: w.scope.clone(),
            device: id,
            seq,
            epoch,
            nonce: STANDARD.encode(nonce),
            ciphertext: STANDARD.encode(ciphertext),
            sig: STANDARD.encode(sig),
        };
        serde_json::to_vec(&envelope).unwrap()
    }

    fn envelope(bytes: &[u8]) -> Map<String, Value> {
        serde_json::from_slice(bytes).unwrap()
    }

    fn raw(w: &World, plaintext: &str) -> Vec<u8> {
        let key = w.key(1);
        seal_json(&w.scope, &w.a.device.sign, 1, 1, &key, plaintext.as_bytes()).unwrap()
    }

    #[test]
    fn a_record_with_a_path_for_a_file_is_skipped_with_a_correct_id() {
        let w = World::new("hostile");
        let mut p = Plaintext::new("t");
        p.records.push(record_in(&ulid(1), "../x.md", &[], "x"));
        p.records.push(record(&ulid(2), &[], "x"));
        p.records.push(record("../../escape", &[], "x"));
        let mut loose = record(&ulid(3), &["p"], "x");
        loose.version.blob = "not-hex".into();
        loose.version.version =
            versions::version_id(&ulid(3), &loose.version.parents, "plan-topic.md", "not-hex");
        p.records.push(loose);
        p.blobs.push(Blob::new(b"x"));
        let read = w.open(&w.seal(&w.a, 1, 1, &p), &w.a, 1).unwrap();
        assert_eq!(read.plaintext.records.len(), 1);
        assert_eq!(read.plaintext.records[0].note, ulid(2));
        assert_eq!(read.skipped.len(), 3);
    }

    #[test]
    fn a_left_record_reads_back_with_the_placeholders() {
        let left = serde_json::json!({
            "note": ulid(1), "version": "c".repeat(64), "event": "left", "at": "t",
            "file": "../../left.md", "blob": "zz"
        });
        let record = Record::try_from(left).unwrap();
        assert_eq!(record.version.file, "");
        assert_eq!(record.version.blob, versions::DELETED);
        let wire = Value::from(record);
        assert!(wire.get("file").is_none() && wire.get("blob").is_none());
    }

    #[test]
    fn a_left_record_needs_hex_ids() {
        let w = World::new("left");
        let bad = |version: &str, parents: &[&str]| {
            serde_json::json!({
                "note": ulid(1), "version": version, "parents": parents, "event": "left", "at": "t"
            })
        };
        let good = "c".repeat(64);
        let json = serde_json::json!({
            "format": 1, "at": "t",
            "records": [bad("not even hex", &[]), bad(&good, &["p"]), bad(&good, &[])],
        });
        let read = w.open(&raw(&w, &json.to_string()), &w.a, 1).unwrap();
        assert_eq!(read.plaintext.records.len(), 1);
        assert_eq!(read.skipped.len(), 2);
    }

    #[test]
    fn a_nonce_of_another_length_is_refused_even_when_it_would_open() {
        let w = World::new("nonce");
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        let env = envelope(&bytes);
        let nonce = STANDARD.decode(env["nonce"].as_str().unwrap()).unwrap();
        let ciphertext = STANDARD
            .decode(env["ciphertext"].as_str().unwrap())
            .unwrap();
        let mut moved = vec![nonce[NONCE_LEN - 1]];
        moved.extend_from_slice(&ciphertext);
        let bytes = forged(&w, 1, 1, &nonce[..NONCE_LEN - 1], &moved);
        let why = w.open(&bytes, &w.a, 1).unwrap_err().to_string();
        assert!(why.contains("24 bytes"), "{why}");
        let same = forged(&w, 1, 1, &nonce, &ciphertext);
        assert!(w.open(&same, &w.a, 1).is_ok());
    }

    #[test]
    fn seq_and_device_are_bound_by_the_signature() {
        let w = World::new("bind");
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        let mut env = envelope(&bytes);
        env["seq"] = Value::from(2);
        let why = w
            .open(&serde_json::to_vec(&env).unwrap(), &w.a, 2)
            .unwrap_err()
            .to_string();
        assert!(why.contains("signature"), "{why}");
        let mut env = envelope(&bytes);
        env["device"] = Value::from(w.b.device.id());
        let why = w
            .open(&serde_json::to_vec(&env).unwrap(), &w.b, 1)
            .unwrap_err()
            .to_string();
        assert!(why.contains("signature"), "{why}");
    }

    #[test]
    fn the_epoch_is_bound_by_the_signature() {
        let w = World::new("epoch-bind");
        w.revoke_b();
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        assert!(w.open(&bytes, &w.a, 1).is_ok());
        let mut env = envelope(&bytes);
        env["epoch"] = Value::from(2);
        let why = w
            .open(&serde_json::to_vec(&env).unwrap(), &w.a, 1)
            .unwrap_err()
            .to_string();
        assert!(why.contains("signature"), "{why}");
    }

    #[test]
    fn a_valid_segment_over_the_limit_is_refused_for_its_size() {
        let w = World::new("size");
        let mut bytes = w.seal(&w.a, 1, 1, &plaintext());
        assert!(w.open(&bytes, &w.a, 1).is_ok());
        bytes.extend(vec![b' '; MAX_BYTES]);
        let why = w.open(&bytes, &w.a, 1).unwrap_err().to_string();
        assert!(why.contains("8 MiB"), "{why}");
    }

    #[test]
    fn an_inner_format_that_differs_is_refused() {
        let w = World::new("inner");
        let mut p = plaintext();
        p.format = 2;
        let why = w
            .open(&w.seal(&w.a, 1, 1, &p), &w.a, 1)
            .unwrap_err()
            .to_string();
        assert!(why.contains("another format"), "{why}");
    }

    #[test]
    fn manifests_of_another_scope_are_refused() {
        let w = World::new("scope-a");
        let other = World::new("scope-b");
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        let id = w.a.device.id();
        let why = open(
            &bytes,
            &w.scope,
            &id,
            1,
            &other.read(),
            &other.opened(&other.a),
        )
        .unwrap_err()
        .to_string();
        assert!(why.contains("another scope"), "{why}");
    }

    #[test]
    fn a_record_that_does_not_parse_is_skipped_and_the_segment_opens() {
        let w = World::new("malformed");
        let good = Value::from(record(&ulid(1), &[], "x"));
        let json = serde_json::json!({
            "format": 1, "at": "t",
            "records": [{"version": "x"}, good, 7],
        });
        let read = w.open(&raw(&w, &json.to_string()), &w.a, 1).unwrap();
        assert_eq!(read.plaintext.records.len(), 1);
        assert_eq!(read.skipped.len(), 2);
        assert!(
            read.skipped[0].why.contains("note"),
            "{:?}",
            read.skipped[0]
        );
    }

    #[test]
    fn only_a_newer_format_asks_for_an_upgrade() {
        let w = World::new("formats");
        let bytes = w.seal(&w.a, 1, 1, &plaintext());
        let with = |format: Value| {
            let mut env = envelope(&bytes);
            env["format"] = format;
            w.open(&serde_json::to_vec(&env).unwrap(), &w.a, 1)
                .unwrap_err()
        };
        assert_eq!(with(Value::from(2)), Refusal::Upgrade(2));
        assert_eq!(with(Value::from(u32::MAX)), Refusal::Upgrade(u32::MAX));
        for bad in [
            Value::from(0),
            Value::from(1u64 << 32),
            Value::from(-1),
            Value::from("1"),
        ] {
            let why = with(bad.clone());
            assert_eq!(
                why,
                Refusal::Failed("its format is not valid".into()),
                "{bad}"
            );
        }
    }
}
