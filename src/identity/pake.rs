//! The pairing code and its exchange: parsing a code, SPAKE2 on both sides, the key schedule, the boxes, the
//! fingerprint and the three mailbox messages. No I/O.

use std::convert::Infallible;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use spake2::rand_core::{CryptoRng, TryCryptoRng, TryRng};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use zeroize::Zeroizing;

use crate::identity::{keys, phrase};
use crate::shared::{hash, store};

/// The format number of every message.
pub const FORMAT: u64 = 1;
/// The most a message may hold, which every reader checks before any other work.
pub const MESSAGE_MAX: usize = 4096;
/// The most scopes one pairing carries.
pub const SCOPES_MAX: usize = 12;

const PROTOCOL: &str = "bilbo-pair-1";
const NONCE_LEN: usize = 24;
/// The prefix of what B signs, so a signature over T cannot stand for any other message of the device key.
const SIGNED: &[u8] = b"bilbo-pair-1 b\n";

/// `keys::random` as the `rand_core` 0.10 RNG `spake2` takes. A failing system source cannot pass through
/// `Infallible`, so it ends the process, as `rand_core`'s `UnwrapErr` does.
pub struct KeysRng;

impl TryRng for KeysRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        let mut bytes = [0u8; 4];
        self.try_fill_bytes(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        let mut bytes = [0u8; 8];
        self.try_fill_bytes(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        for chunk in dst.chunks_mut(32) {
            let bytes = keys::random::<32>().expect("the system random source failed");
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

impl TryCryptoRng for KeysRng {}

/// A pairing code: a nameplate from 1 to 999 and three words of the list.
#[derive(Clone)]
pub struct Code {
    nameplate: u16,
    words: Zeroizing<[u16; 3]>,
}

impl std::fmt::Debug for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Code")
            .field("nameplate", &self.nameplate)
            .finish_non_exhaustive()
    }
}

impl Code {
    /// A code typed in any case, its parts apart by spaces or hyphens, each word in full or by its first four or
    /// more letters. The error names the first word that is not one.
    pub fn parse(typed: &str) -> Result<Code, String> {
        let shape = || {
            "a pairing code is <number>-<word>-<word>-<word>, the number from 1 to 999".to_string()
        };
        let parts: Vec<&str> = typed
            .split(|c: char| c == '-' || c.is_whitespace())
            .filter(|part| !part.is_empty())
            .collect();
        let [number, words @ ..] = parts.as_slice() else {
            return Err(shape());
        };
        if words.len() != 3 || !number.bytes().all(|b| b.is_ascii_digit()) || number.len() > 3 {
            return Err(shape());
        }
        let nameplate: u16 = number.parse().map_err(|_| shape())?;
        if !(1..=999).contains(&nameplate) {
            return Err(shape());
        }
        let mut indexes = Zeroizing::new([0u16; 3]);
        for (slot, word) in indexes.iter_mut().zip(words) {
            *slot =
                phrase::complete(word).ok_or_else(|| format!("'{word}' is not a pairing word"))?;
        }
        Ok(Code {
            nameplate,
            words: indexes,
        })
    }

    /// A random code: the nameplate uniform in 1 to 999, each word uniform over the 2048 of the list.
    pub fn random() -> Result<Code, String> {
        let nameplate = loop {
            let n = u16::from_le_bytes(keys::random::<2>()?) & 0x3ff;
            if (1..=999).contains(&n) {
                break n;
            }
        };
        let mut words = Zeroizing::new([0u16; 3]);
        for slot in words.iter_mut() {
            *slot = u16::from_le_bytes(keys::random::<2>()?) & 0x7ff;
        }
        Ok(Code { nameplate, words })
    }

    /// The nameplate, as the mailbox's folder name.
    pub fn nameplate(&self) -> String {
        self.nameplate.to_string()
    }

    /// The canonical code, lowercase and hyphenated.
    pub fn text(&self) -> String {
        format!(
            "{}-{}-{}-{}",
            self.nameplate,
            phrase::word(self.words[0]),
            phrase::word(self.words[1]),
            phrase::word(self.words[2])
        )
    }

    fn password(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("{PROTOCOL}:{}", self.text()))
    }
}

/// B's identity, as `b.msg`'s box carries it. `owner` is the owner's signing public key, only when B is enrolled.
#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    pub name: String,
    pub sign: [u8; 32],
    pub box_public: [u8; 32],
    pub owner: Option<[u8; 32]>,
}

/// One scope A pairs: what B fetches and checks.
#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub name: String,
    pub id: String,
    /// `any` or `local`.
    pub embedder: String,
    pub n: u64,
    pub hash: [u8; 32],
    pub url: Option<String>,
}

/// `c.msg`'s box on `enrolled`. `seed` is the owner signing seed, left out for an enrolled B.
#[derive(Clone)]
pub struct Payload {
    pub name: String,
    pub id: String,
    pub seed: Option<Zeroizing<[u8; 32]>>,
    pub scopes: Vec<Grant>,
}

impl std::fmt::Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Payload")
            .field("name", &self.name)
            .field("id", &self.id)
            .field("seed", &self.seed.as_ref().map(|_| "<hidden>"))
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// The result `c.msg` carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Enrolled,
    WrongCode,
    Declined,
    Expired,
    NameTaken,
    OtherOwner,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Enrolled => "enrolled",
            Outcome::WrongCode => "wrong-code",
            Outcome::Declined => "declined",
            Outcome::Expired => "expired",
            Outcome::NameTaken => "name-taken",
            Outcome::OtherOwner => "other-owner",
        }
    }

    fn parse(text: &str) -> Option<Outcome> {
        [
            Outcome::Enrolled,
            Outcome::WrongCode,
            Outcome::Declined,
            Outcome::Expired,
            Outcome::NameTaken,
            Outcome::OtherOwner,
        ]
        .into_iter()
        .find(|o| o.as_str() == text)
    }
}

/// Why a message was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A format number this bilbo does not know.
    Newer,
    /// A box that does not open, or a signature that does not verify.
    WrongCode,
    /// Anything else that is not a message of this format.
    Malformed(String),
}

/// What c.msg held.
#[derive(Debug)]
pub enum Reply {
    Enrolled(Payload),
    /// A's owner signing public key.
    OtherOwner([u8; 32]),
    Ended(Outcome),
}

/// What both sides hold once SPAKE2 finishes: the key that seals `c.msg`, the transcript and the fingerprint.
pub struct Session {
    c: Zeroizing<[u8; 32]>,
    transcript: [u8; 32],
    fingerprint: u64,
}

impl Session {
    /// Twelve digits in groups of four, for the two users to compare.
    pub fn fingerprint(&self) -> String {
        let digits = format!("{:012}", self.fingerprint);
        format!("{} {} {}", &digits[..4], &digits[4..8], &digits[8..])
    }
}

/// A's side between `a.msg` and `b.msg`.
pub struct Shown {
    spake: Spake2<Ed25519Group>,
    nameplate: String,
    msg_a: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AMsg {
    format: u64,
    spake: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BMsg {
    format: u64,
    spake: String,
    nonce: String,
    #[serde(rename = "box")]
    sealed: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CMsg {
    format: u64,
    result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
    sealed: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelloWire {
    name: String,
    sign: String,
    box_public: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    sig: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantWire {
    name: String,
    id: String,
    embedder: String,
    n: u64,
    hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PayloadWire {
    name: String,
    id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "write_secret",
        deserialize_with = "read_secret"
    )]
    seed: Option<Zeroizing<String>>,
    scopes: Vec<GrantWire>,
}

fn write_secret<S: Serializer>(
    secret: &Option<Zeroizing<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(text) => serializer.serialize_str(text),
        None => serializer.serialize_none(),
    }
}

fn read_secret<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Zeroizing<String>>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.map(Zeroizing::new))
}

fn malformed(why: &str) -> Refusal {
    Refusal::Malformed(why.to_string())
}

/// A message of this format: refused over `MESSAGE_MAX`, then for its format number, then for any field it
/// should not have.
fn read<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Refusal> {
    if bytes.len() > MESSAGE_MAX {
        return Err(malformed("the message is over 4 KiB"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| malformed("the message is not JSON"))?;
    match value.get("format").and_then(serde_json::Value::as_u64) {
        Some(FORMAT) => {}
        Some(_) => return Err(Refusal::Newer),
        None => return Err(malformed("the message has no format number")),
    }
    serde_json::from_value(value)
        .map_err(|_| malformed("the message does not have the fields of its format"))
}

fn write<T: Serialize>(message: &T) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(message).map_err(|e| format!("cannot write a message: {e}"))?;
    if bytes.len() > MESSAGE_MAX {
        return Err("the message would pass 4 KiB".to_string());
    }
    Ok(bytes)
}

fn spake_bytes(hex: &str) -> Result<[u8; 33], Refusal> {
    keys::unhex::<33>(hex).ok_or_else(|| malformed("the key exchange message is not valid"))
}

fn identities() -> (Identity, Identity) {
    (
        Identity::new(format!("{PROTOCOL} a").as_bytes()),
        Identity::new(format!("{PROTOCOL} b").as_bytes()),
    )
}

/// The three keys and the fingerprint that SPAKE2's key and the two messages give.
struct Schedule {
    transcript: [u8; 32],
    b: Zeroizing<[u8; 32]>,
    c: Zeroizing<[u8; 32]>,
    fingerprint: u64,
}

fn schedule(nameplate: &str, msg_a: &[u8], msg_b: &[u8], key: &[u8]) -> Schedule {
    let mut text = format!("{PROTOCOL}\n{nameplate}\n").into_bytes();
    text.extend_from_slice(msg_a);
    text.extend_from_slice(msg_b);
    let transcript = hash::sha256(&text);
    let fingerprint = keys::hkdf(&transcript, key, b"fingerprint");
    let head: [u8; 8] = fingerprint[..8].try_into().expect("eight bytes");
    Schedule {
        b: keys::hkdf(&transcript, key, b"b"),
        c: keys::hkdf(&transcript, key, b"c"),
        fingerprint: u64::from_be_bytes(head) % 1_000_000_000_000,
        transcript,
    }
}

fn signed(transcript: &[u8; 32]) -> Vec<u8> {
    let mut text = SIGNED.to_vec();
    text.extend_from_slice(transcript);
    text
}

/// A box as its base64 nonce and base64 ciphertext.
fn seal(
    key: &[u8; 32],
    transcript: &[u8; 32],
    plaintext: &[u8],
) -> Result<(String, String), String> {
    let sealed = keys::encrypt(key, transcript, plaintext)?;
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    Ok((STANDARD.encode(nonce), STANDARD.encode(ciphertext)))
}

fn open(
    key: &[u8; 32],
    transcript: &[u8; 32],
    nonce: &str,
    sealed: &str,
) -> Result<Zeroizing<Vec<u8>>, Refusal> {
    let nonce = STANDARD
        .decode(nonce)
        .ok()
        .filter(|n| n.len() == NONCE_LEN)
        .ok_or_else(|| malformed("the nonce is not base64 of 24 bytes"))?;
    let ciphertext = STANDARD
        .decode(sealed)
        .map_err(|_| malformed("the box is not base64"))?;
    let mut data = nonce;
    data.extend_from_slice(&ciphertext);
    keys::decrypt(key, transcript, &data).map_err(|_| Refusal::WrongCode)
}

fn show_with<R: CryptoRng>(code: &Code, rng: R) -> (Shown, Vec<u8>) {
    let (id_a, id_b) = identities();
    let (spake, msg_a) = Spake2::<Ed25519Group>::start_a_with_rng(
        &Password::new(code.password().as_bytes()),
        &id_a,
        &id_b,
        rng,
    );
    let shown = Shown {
        spake,
        nameplate: code.nameplate(),
        msg_a: msg_a.clone(),
    };
    (shown, msg_a)
}

/// A: its SPAKE2 state and `a.msg`.
pub fn show(code: &Code) -> Result<(Shown, Vec<u8>), String> {
    let (shown, msg_a) = show_with(code, KeysRng);
    let message = write(&AMsg {
        format: FORMAT,
        spake: keys::hex(&msg_a),
    })?;
    Ok((shown, message))
}

/// A: finishes SPAKE2 with `b.msg` and opens its box. A box that does not open, or a signature that is not B's
/// over T by the key it names, is `WrongCode`.
pub fn receive(shown: Shown, b_msg: &[u8]) -> Result<(Session, Hello), Refusal> {
    let message: BMsg = read(b_msg)?;
    let msg_b = spake_bytes(&message.spake)?;
    let key = Zeroizing::new(
        shown
            .spake
            .finish(&msg_b)
            .map_err(|_| malformed("the key exchange message is not valid"))?,
    );
    let schedule = schedule(&shown.nameplate, &shown.msg_a, &msg_b, &key);
    let plaintext = open(
        &schedule.b,
        &schedule.transcript,
        &message.nonce,
        &message.sealed,
    )?;
    let wire: HelloWire = serde_json::from_slice(&plaintext)
        .map_err(|_| malformed("the identity in the box is not valid"))?;
    let hello = hello_from(&wire)?;
    let sig =
        keys::unhex::<64>(&wire.sig).ok_or_else(|| malformed("the signature is not valid"))?;
    if !keys::verify(&hello.sign, &signed(&schedule.transcript), &sig) {
        return Err(Refusal::WrongCode);
    }
    let session = Session {
        c: schedule.c,
        transcript: schedule.transcript,
        fingerprint: schedule.fingerprint,
    };
    Ok((session, hello))
}

fn hello_from(wire: &HelloWire) -> Result<Hello, Refusal> {
    let key = |hex: &str| keys::unhex::<32>(hex).ok_or_else(|| malformed("a key is not valid"));
    if !keys::valid_name(&wire.name) {
        return Err(malformed("the device name is not valid"));
    }
    Ok(Hello {
        name: wire.name.clone(),
        sign: key(&wire.sign)?,
        box_public: key(&wire.box_public)?,
        owner: wire.owner.as_deref().map(key).transpose()?,
    })
}

/// B: finishes SPAKE2 with `a.msg` and builds `b.msg`, whose box holds `hello` and B's signature over T.
pub fn answer(
    code: &Code,
    a_msg: &[u8],
    hello: &Hello,
    device: &keys::SignKey,
) -> Result<(Session, Vec<u8>), Refusal> {
    answer_with(code, a_msg, hello, KeysRng, |text| device.sign(text))
}

fn answer_with<R: CryptoRng>(
    code: &Code,
    a_msg: &[u8],
    hello: &Hello,
    rng: R,
    sign: impl FnOnce(&[u8]) -> [u8; 64],
) -> Result<(Session, Vec<u8>), Refusal> {
    let message: AMsg = read(a_msg)?;
    let msg_a = spake_bytes(&message.spake)?;
    if !keys::valid_name(&hello.name) {
        return Err(malformed("the device name is not valid"));
    }
    let (id_a, id_b) = identities();
    let (spake, msg_b) = Spake2::<Ed25519Group>::start_b_with_rng(
        &Password::new(code.password().as_bytes()),
        &id_a,
        &id_b,
        rng,
    );
    let key = Zeroizing::new(
        spake
            .finish(&msg_a)
            .map_err(|_| malformed("the key exchange message is not valid"))?,
    );
    let schedule = schedule(&code.nameplate(), &msg_a, &msg_b, &key);
    let wire = HelloWire {
        name: hello.name.clone(),
        sign: keys::hex(&hello.sign),
        box_public: keys::hex(&hello.box_public),
        owner: hello.owner.as_ref().map(|k| keys::hex(k)),
        sig: keys::hex(&sign(&signed(&schedule.transcript))),
    };
    let plaintext = serde_json::to_vec(&wire).map_err(|e| Refusal::Malformed(e.to_string()))?;
    let (nonce, sealed) =
        seal(&schedule.b, &schedule.transcript, &plaintext).map_err(Refusal::Malformed)?;
    let bytes = write(&BMsg {
        format: FORMAT,
        spake: keys::hex(&msg_b),
        nonce,
        sealed,
    })
    .map_err(Refusal::Malformed)?;
    let session = Session {
        c: schedule.c,
        transcript: schedule.transcript,
        fingerprint: schedule.fingerprint,
    };
    Ok((session, bytes))
}

/// A: `c.msg` for a result that has no box, which needs no session: a wrong code has none.
pub fn plain_reply(outcome: Outcome) -> Result<Vec<u8>, String> {
    if matches!(outcome, Outcome::Enrolled | Outcome::OtherOwner) {
        return Err(format!("a {} reply carries a box", outcome.as_str()));
    }
    write(&CMsg {
        format: FORMAT,
        result: outcome.as_str().to_string(),
        nonce: None,
        sealed: None,
    })
}

/// A: `c.msg`. `payload` goes only with `Enrolled` and `owner` only with `OtherOwner`; both are sealed under
/// the key `c`. It fails rather than write a message over `MESSAGE_MAX`.
pub fn reply(
    session: &Session,
    outcome: Outcome,
    payload: Option<&Payload>,
    owner: Option<&[u8; 32]>,
) -> Result<Vec<u8>, String> {
    let plaintext = match (outcome, payload, owner) {
        (Outcome::Enrolled, Some(payload), None) => payload_bytes(payload)?,
        (Outcome::OtherOwner, None, Some(owner)) => Zeroizing::new(owner.to_vec()),
        (Outcome::Enrolled | Outcome::OtherOwner, _, _) => {
            return Err(format!("a {} reply needs its own box", outcome.as_str()));
        }
        (_, None, None) => return plain_reply(outcome),
        _ => return Err(format!("a {} reply carries no box", outcome.as_str())),
    };
    let (nonce, sealed) = seal(&session.c, &session.transcript, &plaintext)?;
    write(&CMsg {
        format: FORMAT,
        result: outcome.as_str().to_string(),
        nonce: Some(nonce),
        sealed: Some(sealed),
    })
}

fn payload_bytes(payload: &Payload) -> Result<Zeroizing<Vec<u8>>, String> {
    if payload.scopes.len() > SCOPES_MAX {
        return Err(format!("a pairing carries at most {SCOPES_MAX} scopes"));
    }
    let wire = PayloadWire {
        name: payload.name.clone(),
        id: payload.id.clone(),
        seed: payload
            .seed
            .as_ref()
            .map(|s| Zeroizing::new(keys::hex(&**s))),
        scopes: payload
            .scopes
            .iter()
            .map(|g| GrantWire {
                name: g.name.clone(),
                id: g.id.clone(),
                embedder: g.embedder.clone(),
                n: g.n,
                hash: keys::hex(&g.hash),
                url: g.url.clone(),
            })
            .collect(),
    };
    serde_json::to_vec(&wire)
        .map(Zeroizing::new)
        .map_err(|e| format!("cannot write a message: {e}"))
}

fn payload_from(bytes: &[u8]) -> Result<Payload, Refusal> {
    let wire: PayloadWire = serde_json::from_slice(bytes)
        .map_err(|_| malformed("the payload does not have the fields of its format"))?;
    if wire.scopes.len() > SCOPES_MAX || !keys::valid_name(&wire.name) {
        return Err(malformed("the payload is not valid"));
    }
    let seed = match &wire.seed {
        Some(hex) => Some(Zeroizing::new(
            keys::unhex::<32>(hex).ok_or_else(|| malformed("the payload is not valid"))?,
        )),
        None => None,
    };
    let scopes = wire
        .scopes
        .into_iter()
        .map(|g| {
            let hash = keys::unhex::<32>(&g.hash);
            match hash {
                Some(hash)
                    if store::is_topic(&g.name)
                        && matches!(g.embedder.as_str(), "any" | "local") =>
                {
                    Ok(Grant {
                        name: g.name,
                        id: g.id,
                        embedder: g.embedder,
                        n: g.n,
                        hash,
                        url: g.url,
                    })
                }
                _ => Err(malformed("the payload is not valid")),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Payload {
        name: wire.name,
        id: wire.id,
        seed,
        scopes,
    })
}

/// B: reads `c.msg`. A plain result is taken as it stands; a box that does not open is `WrongCode`.
pub fn read_reply(session: &Session, c_msg: &[u8]) -> Result<Reply, Refusal> {
    let message: CMsg = read(c_msg)?;
    let outcome =
        Outcome::parse(&message.result).ok_or_else(|| malformed("the result is not known"))?;
    let boxed = match (&message.nonce, &message.sealed) {
        (Some(nonce), Some(sealed)) => Some((nonce, sealed)),
        (None, None) => None,
        _ => return Err(malformed("the box is incomplete")),
    };
    match (outcome, boxed) {
        (Outcome::Enrolled, Some((nonce, sealed))) => {
            let plaintext = open(&session.c, &session.transcript, nonce, sealed)?;
            payload_from(&plaintext).map(Reply::Enrolled)
        }
        (Outcome::OtherOwner, Some((nonce, sealed))) => {
            let plaintext = open(&session.c, &session.transcript, nonce, sealed)?;
            let owner: [u8; 32] = plaintext
                .as_slice()
                .try_into()
                .map_err(|_| malformed("the owner key is not valid"))?;
            Ok(Reply::OtherOwner(owner))
        }
        (Outcome::Enrolled | Outcome::OtherOwner, None) => Err(malformed("the box is missing")),
        (outcome, None) => Ok(Reply::Ended(outcome)),
        (_, Some(_)) => Err(malformed("this result carries no box")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spake2::{Ed25519Group, Identity, Password, Spake2};

    /// Yields its bytes, then zeros: a scalar's 32 little-endian bytes come out of `Scalar::random` unchanged.
    struct Fixed(Vec<u8>);

    impl TryRng for Fixed {
        type Error = Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Infallible> {
            unreachable!("spake2 draws a scalar with fill_bytes")
        }

        fn try_next_u64(&mut self) -> Result<u64, Infallible> {
            unreachable!("spake2 draws a scalar with fill_bytes")
        }

        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
            let n = dst.len().min(self.0.len());
            dst[..n].copy_from_slice(&self.0[..n]);
            dst[n..].fill(0);
            self.0.drain(..n);
            Ok(())
        }
    }

    impl TryCryptoRng for Fixed {}

    #[test]
    fn the_crates_own_vector_holds() {
        let scalar = |hex: &str| Fixed(keys::unhex::<32>(hex).unwrap().to_vec());
        let a = scalar("25184061a70b1142f1a9f043a52cf7033dc308b5a0a32e42b003ecd59c2ac605");
        let b = scalar("9fb5e845084e0cbe27ac8b4d3af139b33f3f8a047d4234e2d45d33c5cd367b0f");
        let password = Password::new(b"password");
        let (id_a, id_b) = (Identity::new(b"idA"), Identity::new(b"idB"));
        let (side_a, msg_a) = Spake2::<Ed25519Group>::start_a_with_rng(&password, &id_a, &id_b, a);
        let (side_b, msg_b) = Spake2::<Ed25519Group>::start_b_with_rng(&password, &id_a, &id_b, b);
        assert_eq!(
            keys::hex(&msg_a),
            "416fc960df73c9cf8ed7198b0c9534e2e96a5984bfc5edc023fd24dacf371f2af9"
        );
        assert_eq!(
            keys::hex(&msg_b),
            "42354e97b88406922b1df4bea1d7870f17aed3dba7c720b313edae315b00959309"
        );
        let key = side_a.finish(&msg_b).unwrap();
        assert_eq!(side_b.finish(&msg_a).unwrap(), key);
        assert_eq!(
            keys::hex(&key),
            "712295de7219c675ddd31942184aa26e0a957cf216bc230d165b215047b520c1"
        );
    }

    #[test]
    fn keys_rng_draws_fresh_scalars_that_agree() {
        let password = Password::new(b"bilbo-pair-1:42-orbit-tunnel-velvet");
        let (id_a, id_b) = (
            Identity::new(b"bilbo-pair-1 a"),
            Identity::new(b"bilbo-pair-1 b"),
        );
        let (side_a, msg_a) =
            Spake2::<Ed25519Group>::start_a_with_rng(&password, &id_a, &id_b, KeysRng);
        let (side_b, msg_b) =
            Spake2::<Ed25519Group>::start_b_with_rng(&password, &id_a, &id_b, KeysRng);
        let (_, again) = Spake2::<Ed25519Group>::start_a_with_rng(&password, &id_a, &id_b, KeysRng);
        assert_ne!(msg_a, again);
        assert_eq!(
            side_a.finish(&msg_b).unwrap(),
            side_b.finish(&msg_a).unwrap()
        );
    }

    fn code(text: &str) -> Code {
        Code::parse(text).unwrap()
    }

    const CODE: &str = "42-orbit-tunnel-velvet";
    const MSG_A_HEX: &str = "41e63a85cf6c05016244848b55ec05be6f0808ffc5369b2d4a5f95ca39f2ec9628";
    const MSG_B_HEX: &str = "427c7cc57dee528463e81a06700f4f2598bc358b2d6d6fb1dc1cce3f0875ea9790";
    const KEY_HEX: &str = "8e35ceee7ce84b3fc5d67a9dee780a57f9e4ef8c055c3457fab9522fa1eba787";
    const TRANSCRIPT_HEX: &str = "e4b7cb0d866fd4eaaee632cb10724f3be2be122ecab3c3cba8609cb851d33416";
    const B_HEX: &str = "0b1c30ac1f9ff82c8036d9a49b80bbccedb59425c1eb7484070655c5cd9ced0d";
    const C_HEX: &str = "783f5e9b685e08c73deeffeeb1ae09be9717c4413bbfebfeebbd58d04af1a0e1";
    const FINGERPRINT: u64 = 414_160_245_514;

    fn device() -> keys::SignKey {
        keys::SignKey::from_seed(&[7u8; 32])
    }

    fn hello(owner: Option<[u8; 32]>) -> Hello {
        Hello {
            name: "mirkwood".to_string(),
            sign: device().public(),
            box_public: [9u8; 32],
            owner,
        }
    }

    fn scalar(byte: u8) -> Fixed {
        Fixed(vec![byte; 32])
    }

    /// A and B through one exchange, A's side first.
    fn exchange(owner: Option<[u8; 32]>) -> (Session, Hello, Session, Vec<u8>, Vec<u8>) {
        let (shown, a_msg) = show(&code(CODE)).unwrap();
        let (b_session, b_msg) = answer(&code(CODE), &a_msg, &hello(owner), &device()).unwrap();
        let (a_session, got) = receive(shown, &b_msg).unwrap();
        (a_session, got, b_session, a_msg, b_msg)
    }

    fn payload(scopes: usize, name_len: usize) -> Payload {
        Payload {
            name: "rivendell".to_string(),
            id: "a".repeat(26),
            seed: Some(Zeroizing::new([5u8; 32])),
            scopes: (0..scopes)
                .map(|i| Grant {
                    name: format!("{:x<name_len$}", format!("s{i}")),
                    id: "b".repeat(26),
                    embedder: "local".to_string(),
                    n: 3,
                    hash: [i as u8; 32],
                    url: None,
                })
                .collect(),
        }
    }

    fn with_field(message: &[u8], key: &str, value: serde_json::Value) -> Vec<u8> {
        let mut json: serde_json::Value = serde_json::from_slice(message).unwrap();
        json[key] = value;
        serde_json::to_vec(&json).unwrap()
    }

    #[test]
    fn bilbos_golden_key_from_two_fixed_seeds() {
        let code = code(CODE);
        let (id_a, id_b) = identities();
        let password = Password::new(code.password().as_bytes());
        let (side_a, msg_a) =
            Spake2::<Ed25519Group>::start_a_with_rng(&password, &id_a, &id_b, scalar(0x11));
        let (side_b, msg_b) =
            Spake2::<Ed25519Group>::start_b_with_rng(&password, &id_a, &id_b, scalar(0x22));
        let key = side_a.finish(&msg_b).unwrap();
        assert_eq!(side_b.finish(&msg_a).unwrap(), key);
        assert_eq!(KEY_HEX, keys::hex(&key));
        assert_eq!(MSG_A_HEX, keys::hex(&msg_a));
        assert_eq!(MSG_B_HEX, keys::hex(&msg_b));

        let golden = schedule("42", &msg_a, &msg_b, &key);
        assert_eq!(TRANSCRIPT_HEX, keys::hex(&golden.transcript));
        assert_eq!(B_HEX, keys::hex(&*golden.b));
        assert_eq!(C_HEX, keys::hex(&*golden.c));
        assert_eq!(golden.fingerprint, FINGERPRINT);

        // The same seeds through the public functions, which also drive the messages.
        let (shown, a_spake) = show_with(&code, scalar(0x11));
        assert_eq!(a_spake, msg_a);
        let a_msg = write(&AMsg {
            format: FORMAT,
            spake: keys::hex(&a_spake),
        })
        .unwrap();
        let (b_session, b_msg) = answer_with(&code, &a_msg, &hello(None), scalar(0x22), |text| {
            device().sign(text)
        })
        .unwrap();
        let (a_session, _) = receive(shown, &b_msg).unwrap();
        assert_eq!(a_session.transcript, golden.transcript);
        assert_eq!(*b_session.c, *golden.c);
        assert_eq!(a_session.fingerprint(), b_session.fingerprint());
    }

    #[test]
    fn both_sides_agree() {
        let (a, got, b, _, _) = exchange(None);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(*a.c, *b.c);
        assert_eq!(a.transcript, b.transcript);
        assert_eq!(got, hello(None));
    }

    #[test]
    fn an_enrolled_b_names_its_owner() {
        let (_, got, _, _, _) = exchange(Some([3u8; 32]));
        assert_eq!(got.owner, Some([3u8; 32]));
    }

    #[test]
    fn one_wrong_word_does_not_open_b_msgs_box() {
        let (shown, a_msg) = show(&code(CODE)).unwrap();
        let (_, b_msg) = answer(
            &code("42-orbit-tunnel-vacuum"),
            &a_msg,
            &hello(None),
            &device(),
        )
        .unwrap();
        assert_eq!(receive(shown, &b_msg).err(), Some(Refusal::WrongCode));
    }

    #[test]
    fn a_changed_spake_message_opens_neither_box() {
        let (shown, _) = show(&code(CODE)).unwrap();
        let (_, other) = show(&code(CODE)).unwrap();
        // B answers another a.msg of the same code: A cannot open b.msg.
        let (b_session, b_msg) = answer(&code(CODE), &other, &hello(None), &device()).unwrap();
        let (a_session, _) = {
            let (s, m) = show(&code(CODE)).unwrap();
            let (_, b) = answer(&code(CODE), &m, &hello(None), &device()).unwrap();
            receive(s, &b).unwrap()
        };
        assert_eq!(receive(shown, &b_msg).err(), Some(Refusal::WrongCode));
        // A's c.msg does not open for a B whose transcript differs.
        let c = reply(&a_session, Outcome::Enrolled, Some(&payload(1, 8)), None).unwrap();
        assert_eq!(read_reply(&b_session, &c).err(), Some(Refusal::WrongCode));
    }

    #[test]
    fn a_signature_not_over_t_is_refused() {
        let (shown, a_msg) = show(&code(CODE)).unwrap();
        let (_, b_msg) = answer_with(&code(CODE), &a_msg, &hello(None), KeysRng, |text| {
            device().sign(&text[SIGNED.len()..])
        })
        .unwrap();
        assert_eq!(receive(shown, &b_msg).err(), Some(Refusal::WrongCode));
    }

    #[test]
    fn a_signature_not_by_the_key_it_names_is_refused() {
        let (shown, a_msg) = show(&code(CODE)).unwrap();
        let named = Hello {
            sign: keys::SignKey::from_seed(&[8u8; 32]).public(),
            ..hello(None)
        };
        let (_, b_msg) = answer(&code(CODE), &a_msg, &named, &device()).unwrap();
        assert_eq!(receive(shown, &b_msg).err(), Some(Refusal::WrongCode));
    }

    #[test]
    fn a_name_that_is_not_valid_is_refused_on_both_sides() {
        let (_, a_msg) = show(&code(CODE)).unwrap();
        let bad = Hello {
            name: "Not Valid".to_string(),
            ..hello(None)
        };
        assert!(matches!(
            answer(&code(CODE), &a_msg, &bad, &device()),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn an_unknown_format_is_newer_before_anything_else() {
        let (shown, a_msg) = show(&code(CODE)).unwrap();
        let newer_a = with_field(&a_msg, "format", serde_json::json!(2));
        let newer_a = with_field(&newer_a, "extra", serde_json::json!(true));
        assert_eq!(
            answer(&code(CODE), &newer_a, &hello(None), &device()).err(),
            Some(Refusal::Newer)
        );
        let (_, b_msg) = answer(&code(CODE), &a_msg, &hello(None), &device()).unwrap();
        let newer_b = with_field(&b_msg, "format", serde_json::json!(2));
        assert_eq!(receive(shown, &newer_b).err(), Some(Refusal::Newer));
        let (a, ..) = exchange(None);
        let c = plain_reply(Outcome::Declined).unwrap();
        let newer_c = with_field(&c, "format", serde_json::json!(2));
        assert_eq!(read_reply(&a, &newer_c).err(), Some(Refusal::Newer));
    }

    #[test]
    fn a_message_that_is_not_ours_is_refused_before_any_crypto() {
        let (_, a_msg) = show(&code(CODE)).unwrap();
        let junk = [
            b"not json".to_vec(),
            b"[1]".to_vec(),
            br#"{"spake":"00"}"#.to_vec(),
            with_field(&a_msg, "extra", serde_json::json!(1)),
            with_field(&a_msg, "spake", serde_json::json!("41")),
            with_field(&a_msg, "pad", serde_json::json!("x".repeat(MESSAGE_MAX))),
        ];
        for message in junk {
            assert!(
                matches!(
                    answer(&code(CODE), &message, &hello(None), &device()),
                    Err(Refusal::Malformed(_))
                ),
                "{}",
                String::from_utf8_lossy(&message[..message.len().min(40)])
            );
        }
    }

    #[test]
    fn every_result_round_trips() {
        let (a, _, b, _, _) = exchange(None);
        for outcome in [
            Outcome::WrongCode,
            Outcome::Declined,
            Outcome::Expired,
            Outcome::NameTaken,
        ] {
            let c = reply(&a, outcome, None, None).unwrap();
            assert!(matches!(
                read_reply(&b, &c).unwrap(),
                Reply::Ended(got) if got == outcome
            ));
            assert_eq!(c, plain_reply(outcome).unwrap());
        }
        let c = reply(&a, Outcome::OtherOwner, None, Some(&[4u8; 32])).unwrap();
        assert!(matches!(
            read_reply(&b, &c).unwrap(),
            Reply::OtherOwner(owner) if owner == [4u8; 32]
        ));
        let sent = payload(2, 8);
        let c = reply(&a, Outcome::Enrolled, Some(&sent), None).unwrap();
        let Reply::Enrolled(got) = read_reply(&b, &c).unwrap() else {
            panic!("not enrolled");
        };
        assert_eq!(
            (got.name.as_str(), got.id.as_str()),
            ("rivendell", sent.id.as_str())
        );
        assert_eq!(got.scopes, sent.scopes);
        assert_eq!(got.seed.as_deref(), sent.seed.as_deref());
        let enrolled = Payload {
            seed: None,
            ..payload(1, 8)
        };
        let c = reply(&a, Outcome::Enrolled, Some(&enrolled), None).unwrap();
        let Reply::Enrolled(got) = read_reply(&b, &c).unwrap() else {
            panic!("not enrolled");
        };
        assert!(got.seed.is_none());
    }

    #[test]
    fn a_reply_with_the_wrong_box_is_refused() {
        let (a, _, b, _, _) = exchange(None);
        let p = payload(1, 8);
        assert!(reply(&a, Outcome::Enrolled, None, None).is_err());
        assert!(reply(&a, Outcome::Declined, Some(&p), None).is_err());
        assert!(reply(&a, Outcome::OtherOwner, Some(&p), None).is_err());
        assert!(plain_reply(Outcome::Enrolled).is_err());
        let plain = plain_reply(Outcome::Declined).unwrap();
        let boxed = reply(&a, Outcome::OtherOwner, None, Some(&[1u8; 32])).unwrap();
        let swapped = with_field(&plain, "box", serde_json::json!("AAAA"));
        assert!(matches!(
            read_reply(&b, &swapped),
            Err(Refusal::Malformed(_))
        ));
        let no_box = with_field(&boxed, "result", serde_json::json!("enrolled"));
        assert!(matches!(
            read_reply(&b, &no_box),
            Err(Refusal::WrongCode) | Err(Refusal::Malformed(_))
        ));
        let unknown = with_field(&plain, "result", serde_json::json!("maybe"));
        assert!(matches!(
            read_reply(&b, &unknown),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn a_c_msg_for_12_scopes_fits_in_4_kib() {
        let (a, _, b, _, _) = exchange(None);
        let c = reply(&a, Outcome::Enrolled, Some(&payload(SCOPES_MAX, 24)), None).unwrap();
        assert!(c.len() <= MESSAGE_MAX, "{} bytes", c.len());
        assert!(matches!(read_reply(&b, &c).unwrap(), Reply::Enrolled(_)));
        assert!(
            reply(
                &a,
                Outcome::Enrolled,
                Some(&payload(SCOPES_MAX + 1, 24)),
                None
            )
            .is_err()
        );
        assert!(reply(&a, Outcome::Enrolled, Some(&payload(SCOPES_MAX, 400)), None).is_err());
    }

    #[test]
    fn the_fingerprint_is_twelve_digits_in_groups_of_four() {
        let (a, ..) = exchange(None);
        let f = a.fingerprint();
        let groups: Vec<&str> = f.split(' ').collect();
        assert_eq!(groups.len(), 3);
        assert!(
            groups
                .iter()
                .all(|g| g.len() == 4 && g.bytes().all(|b| b.is_ascii_digit()))
        );
    }

    #[test]
    fn a_code_is_read_loosely() {
        for typed in [
            CODE,
            "42 ORBI tunn velvet",
            " 42-Orbit  tunnel-VELV ",
            "042-orbit-tunnel-velvet",
        ] {
            assert_eq!(code(typed).text(), CODE, "{typed}");
        }
        assert_eq!(code(CODE).nameplate(), "42");
    }

    #[test]
    fn a_code_names_the_first_unknown_word() {
        assert_eq!(
            Code::parse("42-orbit-tunel-velvet").unwrap_err(),
            "'tunel' is not a pairing word"
        );
        assert_eq!(
            Code::parse("42-orbi-tunel-vel").unwrap_err(),
            "'tunel' is not a pairing word"
        );
    }

    #[test]
    fn a_code_has_a_number_and_three_words() {
        for typed in [
            "0-orbit-tunnel-velvet",
            "1000-orbit-tunnel-velvet",
            "orbit-tunnel-velvet",
            "42-orbit-tunnel",
            "42-orbit-tunnel-velvet-orbit",
            "x-orbit-tunnel-velvet",
            "",
        ] {
            let err = Code::parse(typed).unwrap_err();
            assert!(
                err.contains("<number>-<word>-<word>-<word>"),
                "{typed}: {err}"
            );
        }
        assert!(Code::parse("999-orbit-tunnel-velvet").is_ok());
        assert!(Code::parse("1-orbit-tunnel-velvet").is_ok());
    }

    #[test]
    fn a_random_code_parses_back_within_range() {
        for _ in 0..300 {
            let code = Code::random().unwrap();
            let n: u16 = code.nameplate().parse().unwrap();
            assert!((1..=999).contains(&n));
            assert_eq!(Code::parse(&code.text()).unwrap().text(), code.text());
        }
    }
}
