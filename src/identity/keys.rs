//! The primitives and key files of a device's identity: encodings, the owner derivation, device keys, signing,
//! sealing, and the folder under `<state>/bilbo/` that holds the secrets.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use hpke::{Deserializable, Kem as _, OpModeR, OpModeS, Serializable};
use serde::Deserialize;
use serde::de::{Deserializer, Visitor};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::host::swap;
use crate::shared::{hash, store};

type Kem = hpke::kem::X25519HkdfSha256;
type Seal = hpke::aead::ChaCha20Poly1305;
type Kdf = hpke::kdf::HkdfSha256;

const OWNER_SALT: &[u8] = b"bilbo-owner-1";
const SEAL_INFO: &[u8] = b"bilbo-epoch-1";
const NONCE_LEN: usize = 24;
const ENC_LEN: usize = 32;
const SEALED_LEN: usize = ENC_LEN + 32 + 16;
const NAME_MAX: usize = 32;

/// A random array from the operating system.
pub fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| format!("cannot read random bytes: {e}"))?;
    Ok(bytes)
}

/// A random 32-byte secret, wiped on drop.
pub fn random_secret() -> Result<Zeroizing<[u8; 32]>, String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut bytes[..]).map_err(|e| format!("cannot read random bytes: {e}"))?;
    Ok(bytes)
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Exactly `N` bytes from lowercase hex; `None` for any other length or character.
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let raw = text.as_bytes();
    if raw.len() != N * 2 {
        return None;
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (byte, pair) in out.iter_mut().zip(raw.chunks(2)) {
        *byte = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// Lowercase RFC 4648 base32 without padding.
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let (mut out, mut buffer, mut bits) = (String::new(), 0u32, 0);
    for &byte in bytes {
        buffer = buffer << 8 | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[(buffer >> bits & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[(buffer << (5 - bits) & 31) as usize] as char);
    }
    out
}

/// A device id: the first 26 base32 characters of the SHA-256 of its Ed25519 public key.
pub fn device_id(sign_public: &[u8; 32]) -> String {
    base32(&hash::sha256(sign_public))[..26].to_string()
}

/// A new scope id: 128 random bits as 26 base32 characters.
pub fn new_scope_id() -> Result<String, String> {
    Ok(base32(&random::<16>()?))
}

/// The owner fingerprint: six groups of four base32 characters of the SHA-256 of the owner's Ed25519 public key.
pub fn owner_fingerprint(sign_public: &[u8; 32]) -> String {
    let text = base32(&hash::sha256(sign_public));
    text.as_bytes()[..24]
        .chunks(4)
        .map(|group| std::str::from_utf8(group).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

/// An Ed25519 signing key, wiped on drop.
pub struct SignKey(SigningKey);

impl SignKey {
    pub fn from_seed(seed: &[u8; 32]) -> SignKey {
        SignKey(SigningKey::from_bytes(seed))
    }

    pub fn public(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }

    pub fn seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }
}

/// Whether `signature` is `public`'s strict Ed25519 signature of `message`.
pub fn verify(public: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public) else {
        return false;
    };
    let signature = ed25519_dalek::Signature::from_bytes(signature);
    key.verify_strict(message, &signature).is_ok()
}

/// An X25519 private key, wiped on drop.
pub struct BoxSecret(Zeroizing<[u8; 32]>);

impl BoxSecret {
    pub fn from_bytes(bytes: &[u8; 32]) -> BoxSecret {
        BoxSecret(Zeroizing::new(*bytes))
    }

    pub fn public(&self) -> [u8; 32] {
        let key = <Kem as hpke::Kem>::PrivateKey::from_bytes(&self.0[..]).expect("32 bytes");
        Kem::sk_to_pk(&key).to_bytes().into()
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The owner's keys, derived from the phrase's entropy and held only while a verb runs.
pub struct Owner {
    pub sign: SignKey,
    pub box_secret: BoxSecret,
}

impl Owner {
    /// HKDF-SHA256 with salt `bilbo-owner-1`: info `sign` is the Ed25519 seed, info `box` the X25519 private key.
    pub fn derive(entropy: &[u8; 16]) -> Owner {
        let hkdf = Hkdf::<Sha256>::new(Some(OWNER_SALT), entropy);
        let mut sign = Zeroizing::new([0u8; 32]);
        let mut boxed = Zeroizing::new([0u8; 32]);
        hkdf.expand(b"sign", &mut sign[..]).expect("32 bytes");
        hkdf.expand(b"box", &mut boxed[..]).expect("32 bytes");
        Owner {
            sign: SignKey::from_seed(&sign),
            box_secret: BoxSecret::from_bytes(&boxed),
        }
    }

    /// What `owner.key` keeps of it: the signing seed and the box public key, never the box secret.
    pub fn file(&self) -> OwnerFile {
        OwnerFile {
            sign: SignKey::from_seed(&self.sign.seed()),
            box_public: self.box_secret.public(),
        }
    }
}

/// The contents of `owner.key`.
pub struct OwnerFile {
    pub sign: SignKey,
    pub box_public: [u8; 32],
}

/// This device's own key pair and name.
pub struct Device {
    pub name: String,
    pub sign: SignKey,
    pub box_secret: BoxSecret,
}

impl Device {
    /// A device from 64 random bytes.
    pub fn generate(name: &str) -> Result<Device, String> {
        Ok(Device::from_seeds(
            name,
            &*random_secret()?,
            &*random_secret()?,
        ))
    }

    pub fn from_seeds(name: &str, sign_seed: &[u8; 32], box_secret: &[u8; 32]) -> Device {
        Device {
            name: name.to_string(),
            sign: SignKey::from_seed(sign_seed),
            box_secret: BoxSecret::from_bytes(box_secret),
        }
    }

    pub fn id(&self) -> String {
        device_id(&self.sign.public())
    }
}

fn epoch_aad(scope: &str, epoch: u64, recipient: &str) -> Vec<u8> {
    format!("{scope}\n{epoch}\n{recipient}").into_bytes()
}

/// An epoch key sealed to `box_public` with HPKE base mode: the encapsulated key, then the ciphertext.
pub fn seal_epoch(
    box_public: &[u8; 32],
    scope: &str,
    epoch: u64,
    recipient: &str,
    key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    let public = <Kem as hpke::Kem>::PublicKey::from_bytes(box_public)
        .map_err(|e| format!("not an X25519 public key: {e}"))?;
    let aad = epoch_aad(scope, epoch, recipient);
    let (enc, ciphertext) =
        hpke::single_shot_seal::<Seal, Kdf, Kem>(&OpModeS::Base, &public, SEAL_INFO, key, &aad)
            .map_err(|e| format!("cannot seal the epoch key: {e}"))?;
    let mut sealed = enc.to_bytes().to_vec();
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

/// The epoch key a sealed value holds for `recipient`, or why it does not open.
pub fn open_epoch(
    secret: &BoxSecret,
    scope: &str,
    epoch: u64,
    recipient: &str,
    sealed: &[u8],
) -> Result<Zeroizing<[u8; 32]>, String> {
    let fail = || "the epoch key does not open with this key".to_string();
    if sealed.len() != SEALED_LEN {
        return Err(fail());
    }
    let key = <Kem as hpke::Kem>::PrivateKey::from_bytes(&secret.0[..]).map_err(|_| fail())?;
    let enc =
        <Kem as hpke::Kem>::EncappedKey::from_bytes(&sealed[..ENC_LEN]).map_err(|_| fail())?;
    let aad = epoch_aad(scope, epoch, recipient);
    let plain = hpke::single_shot_open::<Seal, Kdf, Kem>(
        &OpModeR::Base,
        &key,
        &enc,
        SEAL_INFO,
        &sealed[ENC_LEN..],
        &aad,
    )
    .map_err(|_| fail())?;
    let plain = Zeroizing::new(plain);
    <[u8; 32]>::try_from(&plain[..])
        .map(Zeroizing::new)
        .map_err(|_| fail())
}

/// The aad of a scope name's ciphertext.
pub fn name_aad(scope: &str, epoch: u64) -> Vec<u8> {
    format!("bilbo-name-1\n{scope}\n{epoch}").into_bytes()
}

/// The aad of chain entry `epoch`'s ciphertext.
pub fn chain_aad(scope: &str, epoch: u64) -> Vec<u8> {
    format!("bilbo-chain-1\n{scope}\n{epoch}").into_bytes()
}

/// XChaCha20-Poly1305 under `key` with a random nonce: the nonce, then the ciphertext.
pub fn encrypt(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).expect("32 bytes");
    let nonce = random::<NONCE_LEN>()?;
    let ciphertext = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| "cannot encrypt".to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// The plaintext of `encrypt`'s output, or why it does not open.
pub fn decrypt(key: &[u8; 32], aad: &[u8], data: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    if data.len() <= NONCE_LEN {
        return Err("the ciphertext is too short".into());
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key).expect("32 bytes");
    let (nonce, ciphertext) = data.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at the nonce length");
    cipher
        .decrypt(
            (&nonce).into(),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| "the ciphertext does not open with this key".to_string())
}

/// Whether `name` follows the device name rule: the topic grammar, at most 32 characters.
pub fn valid_name(name: &str) -> bool {
    name.len() <= NAME_MAX && store::is_topic(name)
}

/// A host name as a device name: up to its first `.`, lowercased, each run of other characters one hyphen.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let first = raw.split('.').next().unwrap_or("");
    let mut out = String::new();
    for ch in first.chars().map(|c| c.to_ascii_lowercase()) {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(NAME_MAX);
    let name = out.trim_matches('-');
    (!name.is_empty()).then(|| name.to_string())
}

/// This host's name as a device name; `None` when the call fails or no letter or digit is left.
pub fn host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is writable for its whole length, which is what gethostname is told.
    let rc = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if rc != 0 {
        return None;
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    sanitize_name(&String::from_utf8_lossy(&buffer[..end]))
}

/// Stops this process from writing a core file, so a crash cannot leave a phrase on disk.
pub fn no_core_dump() -> Result<(), String> {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid rlimit that outlives the call.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
        return Err(format!(
            "cannot turn core files off: {}",
            std::io::Error::last_os_error()
        ));
    }
    #[cfg(target_os = "linux")]
    // SAFETY: PR_SET_DUMPABLE takes an integer and reads no memory.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
        return Err(format!(
            "cannot make the process undumpable: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// The key folder's lock and staging folder, beside it.
fn lock_path(keys: &Path) -> PathBuf {
    keys.with_file_name("keys.lock")
}

/// `keys.new`, the folder an identity is built in before it moves to `keys`.
pub fn staging_path(keys: &Path) -> PathBuf {
    keys.with_file_name("keys.new")
}

/// The leftover of an interrupted write, when there is one.
pub fn leftover(keys: &Path) -> Option<PathBuf> {
    let path = staging_path(keys);
    path.symlink_metadata().is_ok().then_some(path)
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(fail)?;
    file.write_all(text.as_bytes()).map_err(fail)?;
    file.sync_all().map_err(fail)
}

/// Writes `owner.key` and `device.key` into `keys`, whole or not at all, never over an existing identity.
pub fn write_identity(keys: &Path, owner: &OwnerFile, device: &Device) -> Result<(), String> {
    if !valid_name(&device.name) {
        return Err(format!("'{}' is not a device name", device.name));
    }
    let parent = keys.parent().ok_or("the key folder has no parent")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(lock_path(keys))
        .map_err(|e| format!("cannot open {}: {e}", lock_path(keys).display()))?;
    lock.lock()
        .map_err(|e| format!("cannot lock {}: {e}", lock_path(keys).display()))?;
    if keys.symlink_metadata().is_ok() {
        return Err(format!("{} already holds an identity", keys.display()));
    }
    let staging = staging_path(keys);
    if let Ok(meta) = staging.symlink_metadata() {
        let removed = if meta.is_dir() {
            std::fs::remove_dir_all(&staging)
        } else {
            std::fs::remove_file(&staging)
        };
        removed.map_err(|e| format!("cannot remove {}: {e}", staging.display()))?;
    }
    let mut builder = std::fs::DirBuilder::new();
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(&staging)
        .map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
    let mut owner_text = Zeroizing::new(String::from("{\"format\":1,\"sign\":\""));
    push_hex(&mut owner_text, &owner.sign.seed()[..]);
    owner_text.push_str("\",\"box_public\":\"");
    push_hex(&mut owner_text, &owner.box_public);
    owner_text.push_str("\"}\n");
    let mut device_text = Zeroizing::new(format!(
        "{{\"format\":1,\"name\":\"{}\",\"sign\":\"",
        device.name
    ));
    push_hex(&mut device_text, &device.sign.seed()[..]);
    device_text.push_str("\",\"box\":\"");
    push_hex(&mut device_text, device.box_secret.bytes());
    device_text.push_str("\"}\n");
    write_file(&staging.join("owner.key"), &owner_text)?;
    write_file(&staging.join("device.key"), &device_text)?;
    File::open(&staging)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| format!("cannot sync {}: {e}", staging.display()))?;
    swap::rename_new(&staging, keys)?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| format!("cannot sync {}: {e}", parent.display()))
}

/// Appends lowercase hex to a wiped string, so a secret never passes through a plain one.
fn push_hex(out: &mut Zeroizing<String>, bytes: &[u8]) {
    use std::fmt::Write as _;
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
}

/// A 32-byte value read from hex, wiped on drop.
struct Key32(Zeroizing<[u8; 32]>);

impl<'de> Deserialize<'de> for Key32 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Key32, D::Error> {
        struct Hex;
        impl Visitor<'_> for Hex {
            type Value = Key32;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("64 lowercase hexadecimal characters")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Key32, E> {
                unhex::<32>(text)
                    .map(|bytes| Key32(Zeroizing::new(bytes)))
                    .ok_or_else(|| E::custom("not 64 lowercase hexadecimal characters"))
            }
        }
        deserializer.deserialize_str(Hex)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerJson {
    format: u64,
    sign: Key32,
    box_public: Key32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceJson {
    format: u64,
    name: String,
    sign: Key32,
    #[serde(rename = "box")]
    boxed: Key32,
}

/// An enrolled device's keys.
pub struct Identity {
    pub owner: OwnerFile,
    pub device: Device,
}

fn loose(path: &Path, want: u32, kind: &str) -> Result<(), String> {
    let meta = path
        .symlink_metadata()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let wanted_kind = if kind == "folder" {
        meta.is_dir()
    } else {
        meta.is_file()
    };
    if !wanted_kind {
        return Err(format!("{} is not a {kind}", path.display()));
    }
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(format!(
            "{} is open to other users (mode {:o}); run chmod {want:o} {}",
            path.display(),
            mode & 0o777,
            path.display()
        ));
    }
    Ok(())
}

fn damaged(path: &Path, keys: &Path, why: &str) -> String {
    format!(
        "{} is damaged: {why}; move {} aside and run bilbo device recover",
        path.display(),
        keys.display()
    )
}

fn read_secret(path: &Path) -> Result<Zeroizing<String>, String> {
    loose(path, 0o600, "file")?;
    std::fs::read_to_string(path)
        .map(Zeroizing::new)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// The keys in `keys`, `None` when the folder is absent. Refuses loose modes and a damaged identity.
pub fn read_identity(keys: &Path) -> Result<Option<Identity>, String> {
    if keys.symlink_metadata().is_err() {
        return Ok(None);
    }
    loose(keys, 0o700, "folder")?;
    let owner_path = keys.join("owner.key");
    let device_path = keys.join("device.key");
    if !owner_path.exists() || !device_path.exists() {
        return Err(damaged(
            keys,
            keys,
            "it must hold both owner.key and device.key",
        ));
    }
    let owner_text = read_secret(&owner_path)?;
    let owner: OwnerJson = serde_json::from_str(&owner_text)
        .map_err(|e| damaged(&owner_path, keys, &e.to_string()))?;
    if owner.format != 1 {
        return Err(damaged(&owner_path, keys, "it is not format 1"));
    }
    let device_text = read_secret(&device_path)?;
    let device: DeviceJson = serde_json::from_str(&device_text)
        .map_err(|e| damaged(&device_path, keys, &e.to_string()))?;
    if device.format != 1 {
        return Err(damaged(&device_path, keys, "it is not format 1"));
    }
    if !valid_name(&device.name) {
        return Err(damaged(
            &device_path,
            keys,
            "the name breaks the device name rule",
        ));
    }
    Ok(Some(Identity {
        owner: OwnerFile {
            sign: SignKey::from_seed(&owner.sign.0),
            box_public: *owner.box_public.0,
        },
        device: Device::from_seeds(&device.name, &device.sign.0, &device.boxed.0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-keys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    const ABANDON: [u8; 16] = [0; 16];

    fn owner() -> Owner {
        Owner::derive(&ABANDON)
    }

    fn device(seed: u8) -> Device {
        Device::from_seeds("rivendell", &[seed; 32], &[seed.wrapping_add(1); 32])
    }

    #[test]
    fn base32_rfc4648_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "my"),
            ("fo", "mzxq"),
            ("foo", "mzxw6"),
            ("foob", "mzxw6yq"),
            ("fooba", "mzxw6ytb"),
            ("foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(base32(plain.as_bytes()), coded);
        }
    }

    #[test]
    fn hex_round_trips_and_refuses_the_rest() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(unhex::<3>("000fff"), Some([0, 15, 255]));
        assert_eq!(unhex::<3>("000FFF"), None);
        assert_eq!(unhex::<3>("000ff"), None);
        assert_eq!(unhex::<3>("000fgf"), None);
    }

    #[test]
    fn the_known_owner() {
        let owner = owner();
        assert_eq!(
            hex(&owner.sign.public()),
            "12a801fee3d44e9780f252b49c3727e88ccec5d8af61a84219a2e917d9217c79"
        );
        assert_eq!(
            hex(&owner.box_secret.public()),
            "11985d0260365b681492a704eb1fbba906c0a9729a731b3428ad884b6d118b60"
        );
        assert_eq!(
            owner_fingerprint(&owner.sign.public()),
            "yb4b-5aju-v6zb-x2nm-nc5x-ompf"
        );
    }

    #[test]
    fn another_entropy_gives_another_fingerprint() {
        let mut entropy = ABANDON;
        entropy[15] = 1;
        let other = Owner::derive(&entropy);
        assert_ne!(
            owner_fingerprint(&other.sign.public()),
            owner_fingerprint(&owner().sign.public())
        );
    }

    #[test]
    fn ids_have_their_forms() {
        let id = device_id(&device(1).sign.public());
        assert_eq!(id.len(), 26);
        assert!(id.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')));
        assert_eq!(new_scope_id().unwrap().len(), 26);
        assert_ne!(new_scope_id().unwrap(), new_scope_id().unwrap());
        assert_ne!(
            Device::generate("a").unwrap().id(),
            Device::generate("a").unwrap().id()
        );
    }

    #[test]
    fn signatures_verify_strictly_and_tampering_fails() {
        let owner = owner();
        let signature = owner.sign.sign(b"message");
        assert!(verify(&owner.sign.public(), b"message", &signature));
        assert!(!verify(&owner.sign.public(), b"messagf", &signature));
        let mut bad = signature;
        bad[0] ^= 1;
        assert!(!verify(&owner.sign.public(), b"message", &bad));
        assert!(!verify(&device(1).sign.public(), b"message", &signature));
    }

    #[test]
    fn sealing_round_trips_to_a_random_and_to_the_owner_box_key() {
        let key = random_secret().unwrap();
        let random = BoxSecret::from_bytes(&random_secret().unwrap());
        let sealed = seal_epoch(&random.public(), "scope", 1, "dev", &key).unwrap();
        assert_eq!(sealed.len(), 80);
        let opened = open_epoch(&random, "scope", 1, "dev", &sealed).unwrap();
        assert_eq!(*opened, *key);

        let owner = owner();
        let sealed = seal_epoch(&owner.box_secret.public(), "scope", 2, "owner", &key).unwrap();
        let opened = open_epoch(&owner.box_secret, "scope", 2, "owner", &sealed).unwrap();
        assert_eq!(*opened, *key);
    }

    #[test]
    fn sealing_refuses_a_wrong_key_aad_recipient_or_truncation() {
        let key = random_secret().unwrap();
        let right = BoxSecret::from_bytes(&random_secret().unwrap());
        let wrong = BoxSecret::from_bytes(&random_secret().unwrap());
        let sealed = seal_epoch(&right.public(), "scope", 1, "dev", &key).unwrap();
        assert!(open_epoch(&wrong, "scope", 1, "dev", &sealed).is_err());
        assert!(open_epoch(&right, "other", 1, "dev", &sealed).is_err());
        assert!(open_epoch(&right, "scope", 2, "dev", &sealed).is_err());
        assert!(open_epoch(&right, "scope", 1, "other", &sealed).is_err());
        assert!(open_epoch(&right, "scope", 1, "dev", &sealed[..40]).is_err());
        assert!(open_epoch(&right, "scope", 1, "dev", &[]).is_err());
    }

    #[test]
    fn symmetric_encryption_round_trips_and_binds_its_aad() {
        let key = random_secret().unwrap();
        let aad = name_aad("scope", 1);
        let sealed = encrypt(&key, &aad, b"personal").unwrap();
        assert_ne!(sealed, encrypt(&key, &aad, b"personal").unwrap());
        assert_eq!(&decrypt(&key, &aad, &sealed).unwrap()[..], b"personal");
        assert!(decrypt(&key, &name_aad("scope", 2), &sealed).is_err());
        assert!(decrypt(&key, &chain_aad("scope", 1), &sealed).is_err());
        assert!(decrypt(&[0; 32], &aad, &sealed).is_err());
        assert!(decrypt(&key, &aad, &sealed[..20]).is_err());
        assert_eq!(chain_aad("s", 3), b"bilbo-chain-1\ns\n3");
        assert_eq!(name_aad("s", 3), b"bilbo-name-1\ns\n3");
    }

    #[test]
    fn host_names_are_sanitized() {
        for (raw, name) in [
            ("Daniels-MacBook-Pro.local", Some("daniels-macbook-pro")),
            ("Bag_End", Some("bag-end")),
            ("a  b__c", Some("a-b-c")),
            ("--rivendell--", Some("rivendell")),
            ("Frodo's Mac.example.org", Some("frodo-s-mac")),
            ("ünï", Some("n")),
            ("___", None),
            ("", None),
            (".local", None),
        ] {
            assert_eq!(sanitize_name(raw).as_deref(), name, "{raw}");
        }
        let long = sanitize_name(&format!("{}-{}", "a".repeat(31), "b".repeat(5))).unwrap();
        assert_eq!(long, "a".repeat(31));
        assert!(valid_name(&long));
    }

    #[test]
    fn the_name_rule() {
        assert!(valid_name("rivendell"));
        assert!(valid_name("a-b-2"));
        assert!(!valid_name("Bag_End"));
        assert!(!valid_name("a--b"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(33)));
        assert!(valid_name(&"a".repeat(32)));
    }

    #[test]
    fn the_host_has_a_name_or_none() {
        if let Some(name) = host_name() {
            assert!(valid_name(&name));
        }
    }

    #[test]
    fn core_files_are_turned_off() {
        no_core_dump().unwrap();
        let mut limit = libc::rlimit {
            rlim_cur: 1,
            rlim_max: 1,
        };
        // SAFETY: `limit` is a valid rlimit that outlives the call.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) }, 0);
        assert_eq!((limit.rlim_cur, limit.rlim_max), (0, 0));
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn keys_path(dir: &Scratch) -> PathBuf {
        dir.0.join("bilbo/keys")
    }

    fn written(dir: &Scratch) -> PathBuf {
        let keys = keys_path(dir);
        write_identity(&keys, &owner().file(), &device(7)).unwrap();
        keys
    }

    #[test]
    fn written_keys_have_their_modes_and_read_back() {
        let dir = scratch("modes");
        let keys = written(&dir);
        assert_eq!(mode(&keys), 0o700);
        assert_eq!(mode(&keys.join("owner.key")), 0o600);
        assert_eq!(mode(&keys.join("device.key")), 0o600);
        assert!(leftover(&keys).is_none());
        let identity = read_identity(&keys).unwrap().unwrap();
        assert_eq!(identity.device.name, "rivendell");
        assert_eq!(identity.device.id(), device(7).id());
        assert_eq!(identity.owner.sign.public(), owner().sign.public());
        assert_eq!(identity.owner.box_public, owner().box_secret.public());
        assert_eq!(
            identity.device.box_secret.public(),
            device(7).box_secret.public()
        );
    }

    #[test]
    fn owner_key_holds_no_box_secret() {
        let dir = scratch("nosecret");
        let keys = written(&dir);
        let text = std::fs::read_to_string(keys.join("owner.key")).unwrap();
        assert!(text.contains(&hex(&owner().box_secret.public())));
        assert!(!text.contains(&hex(owner().box_secret.bytes())));
        assert!(text.contains(&hex(&owner().sign.seed()[..])));
    }

    #[test]
    fn a_second_write_is_refused_and_changes_nothing() {
        let dir = scratch("second");
        let keys = written(&dir);
        let before = std::fs::read(keys.join("device.key")).unwrap();
        let err = write_identity(&keys, &owner().file(), &device(9)).unwrap_err();
        assert!(err.contains("already holds an identity"), "{err}");
        assert_eq!(std::fs::read(keys.join("device.key")).unwrap(), before);
    }

    #[test]
    fn a_bad_name_writes_nothing() {
        let dir = scratch("badname");
        let keys = keys_path(&dir);
        let bad = Device::from_seeds("Bag_End", &[1; 32], &[2; 32]);
        assert!(write_identity(&keys, &owner().file(), &bad).is_err());
        assert!(!dir.0.join("bilbo").exists());
    }

    #[test]
    fn a_leftover_is_reported_then_removed_by_the_next_write() {
        let dir = scratch("leftover");
        let keys = keys_path(&dir);
        let staging = staging_path(&keys);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("stale.key"), "x").unwrap();
        assert_eq!(read_identity(&keys).unwrap().map(|_| ()), None);
        assert_eq!(leftover(&keys), Some(staging.clone()));
        assert!(staging.join("stale.key").exists());
        write_identity(&keys, &owner().file(), &device(7)).unwrap();
        assert!(leftover(&keys).is_none());
        assert!(read_identity(&keys).unwrap().is_some());
    }

    #[test]
    fn no_folder_is_no_identity() {
        let dir = scratch("none");
        assert!(read_identity(&keys_path(&dir)).unwrap().is_none());
    }

    fn chmod(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn loose_modes_are_refused_naming_the_path() {
        let dir = scratch("loose");
        let keys = written(&dir);
        chmod(&keys, 0o755);
        let err = read_identity(&keys).err().unwrap();
        assert!(err.contains(&keys.display().to_string()), "{err}");
        assert!(err.contains("chmod"), "{err}");
        chmod(&keys, 0o700);
        let file = keys.join("device.key");
        chmod(&file, 0o640);
        let err = read_identity(&keys).err().unwrap();
        assert!(err.contains(&file.display().to_string()), "{err}");
        chmod(&file, 0o600);
        assert!(read_identity(&keys).unwrap().is_some());
        chmod(&keys.join("owner.key"), 0o604);
        assert!(read_identity(&keys).is_err());
    }

    #[test]
    fn a_damaged_identity_is_refused() {
        let dir = scratch("damaged");
        let keys = written(&dir);
        let device_key = keys.join("device.key");
        let good = std::fs::read_to_string(&device_key).unwrap();

        std::fs::remove_file(&device_key).unwrap();
        let err = read_identity(&keys).err().unwrap();
        assert!(err.contains("both owner.key and device.key"), "{err}");

        for broken in [
            good.replace("rivendell", "Bag_End"),
            good.replacen("\"sign\":\"", "\"sign\":\"0", 1),
            good.replace("\"name\"", "\"nom\""),
            "not json".to_string(),
        ] {
            std::fs::write(&device_key, broken).unwrap();
            chmod(&device_key, 0o600);
            let err = read_identity(&keys).err().unwrap();
            assert!(err.contains(&device_key.display().to_string()), "{err}");
            assert!(err.contains("bilbo device recover"), "{err}");
        }

        std::fs::write(&device_key, &good).unwrap();
        chmod(&device_key, 0o600);
        let owner_key = keys.join("owner.key");
        let owner_good = std::fs::read_to_string(&owner_key).unwrap();
        std::fs::write(&owner_key, owner_good.replace("\"box_public\"", "\"box\"")).unwrap();
        chmod(&owner_key, 0o600);
        assert!(read_identity(&keys).is_err());
    }

    #[test]
    fn two_writers_leave_one_identity() {
        let dir = scratch("race");
        let keys = keys_path(&dir);
        let results: Vec<_> = (0..4u8)
            .map(|i| {
                let keys = keys.clone();
                std::thread::spawn(move || write_identity(&keys, &owner().file(), &device(i + 1)))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(read_identity(&keys).unwrap().is_some());
        assert!(leftover(&keys).is_none());
    }

    #[test]
    fn the_wire_formats_are_pinned() {
        let owner = owner();
        let sealed = unhex::<80>(concat!(
            "66831387a5d4b410a1e6fd07d5ec1ed4fa4055640ad05222c653f6f05b2d774a",
            "59e8a010e3fc90f389ba14691f98d4039995642c50c687b02a75578689a01551",
            "89e39e183e9f22b59db8676fde9abd0c"
        ))
        .unwrap();
        let key = open_epoch(&owner.box_secret, "s", 1, "owner", &sealed).unwrap();
        assert_eq!(*key, [7; 32]);
        let name = unhex::<48>(concat!(
            "51cf4dd4e3c6dcbd2be4bb9e16bf056c1af8a085e5b9382ac37b7445d6b283ea",
            "b71b2f3c71fca77297284e545e45d852"
        ))
        .unwrap();
        assert_eq!(
            &decrypt(&[7; 32], &name_aad("s", 1), &name).unwrap()[..],
            b"personal"
        );
        assert_eq!(
            hex(&owner.sign.sign(b"bilbo-manifest-1\n{}")),
            concat!(
                "10677e6c9bb87066035ec3a42b97e90e3bfacb34d8e06fabedfb6ab1f6f065d3",
                "db3ab18886e9a27fccc80f141da6704fb77ca8e979c96f34a20f47ee3f3fe507"
            )
        );
    }

    #[test]
    fn the_device_id_is_pinned_for_fixed_seeds() {
        let device = Device::from_seeds("rivendell", &[1; 32], &[2; 32]);
        assert_eq!(
            hex(&device.sign.public()),
            "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c"
        );
        assert_eq!(device.id(), "gr2q7gf5lh6pzfdnurnkvputhp");
    }

    #[test]
    fn the_identity_point_never_verifies() {
        let mut point = [0u8; 32];
        point[0] = 1;
        let mut signature = [0u8; 64];
        signature[0] = 1;
        assert!(!verify(&point, b"anything", &signature));
    }

    #[test]
    fn a_sealed_value_must_be_exactly_80_bytes() {
        let key = random_secret().unwrap();
        let secret = BoxSecret::from_bytes(&random_secret().unwrap());
        let mut sealed = seal_epoch(&secret.public(), "s", 1, "d", &key).unwrap();
        sealed.push(0);
        assert!(open_epoch(&secret, "s", 1, "d", &sealed).is_err());
    }

    #[test]
    fn a_leftover_that_is_a_file_or_a_link_is_removed_too() {
        for link in [false, true] {
            let dir = scratch(if link { "leftlink" } else { "leftfile" });
            let keys = keys_path(&dir);
            std::fs::create_dir_all(keys.parent().unwrap()).unwrap();
            if link {
                std::os::unix::fs::symlink("/nonexistent", staging_path(&keys)).unwrap();
            } else {
                std::fs::write(staging_path(&keys), "x").unwrap();
            }
            write_identity(&keys, &owner().file(), &device(7)).unwrap();
            assert!(leftover(&keys).is_none());
            assert!(read_identity(&keys).unwrap().is_some());
        }
    }

    #[test]
    fn key_files_start_with_their_format() {
        let dir = scratch("format");
        let keys = written(&dir);
        for file in ["owner.key", "device.key"] {
            let text = std::fs::read_to_string(keys.join(file)).unwrap();
            assert!(text.starts_with("{\"format\":1,"), "{text}");
        }
    }

    #[test]
    fn a_missing_or_other_format_is_a_damaged_identity() {
        let dir = scratch("otherformat");
        let keys = written(&dir);
        for file in ["owner.key", "device.key"] {
            let path = keys.join(file);
            let good = std::fs::read_to_string(&path).unwrap();
            for broken in [
                good.replace("\"format\":1,", ""),
                good.replace("\"format\":1", "\"format\":2"),
            ] {
                std::fs::write(&path, broken).unwrap();
                chmod(&path, 0o600);
                let err = read_identity(&keys).err().unwrap();
                assert!(err.contains(&path.display().to_string()), "{err}");
            }
            std::fs::write(&path, good).unwrap();
            chmod(&path, 0o600);
        }
        assert!(read_identity(&keys).unwrap().is_some());
    }

    #[test]
    fn a_symlinked_key_file_or_folder_is_refused() {
        let dir = scratch("symlinks");
        let keys = written(&dir);
        let outside = dir.0.join("outside.key");
        let device_key = keys.join("device.key");
        std::fs::rename(&device_key, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &device_key).unwrap();
        let err = read_identity(&keys).err().unwrap();
        assert!(err.contains("not a file"), "{err}");

        let moved = dir.0.join("moved");
        std::fs::rename(&keys, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &keys).unwrap();
        let err = read_identity(&keys).err().unwrap();
        assert!(err.contains("not a folder"), "{err}");
    }

    #[test]
    fn an_unknown_member_is_a_damaged_identity() {
        let dir = scratch("unknown");
        let keys = written(&dir);
        for file in ["owner.key", "device.key"] {
            let path = keys.join(file);
            let good = std::fs::read_to_string(&path).unwrap();
            std::fs::write(
                &path,
                good.replace("{\"format\":1,", "{\"format\":1,\"extra\":1,"),
            )
            .unwrap();
            chmod(&path, 0o600);
            assert!(read_identity(&keys).is_err(), "{file}");
            std::fs::write(&path, good).unwrap();
            chmod(&path, 0o600);
        }
    }

    #[test]
    fn a_damaged_keys_folder_names_itself_to_move() {
        let dir = scratch("movefolder");
        let keys = written(&dir);
        std::fs::remove_file(keys.join("owner.key")).unwrap();
        let err = read_identity(&keys).err().unwrap();
        assert!(
            err.contains(&format!("move {} aside", keys.display())),
            "{err}"
        );
    }
}
