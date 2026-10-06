//! The request signature both ends of a relay share: Ed25519 over `bilbo-relay-1`, the method, the target, the time,
//! the nonce and the body's SHA-256, carried in four hex headers.

use crate::identity::keys::SignKey;

/// The first line of every signed text, so a request signature never verifies as a manifest or segment signature.
pub const DOMAIN: &str = "bilbo-relay-1";

/// How far a request's time may be from the relay's clock, in seconds.
pub const WINDOW: u64 = 300;

/// The four headers, in lowercase as `http::Request::header` takes them.
pub const KEY: &str = "bilbo-key";
pub const TIME: &str = "bilbo-time";
pub const NONCE: &str = "bilbo-nonce";
pub const SIGNATURE: &str = "bilbo-signature";

/// A request's signature headers, decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Signed {
    pub key: [u8; 32],
    pub time: u64,
    pub nonce: [u8; 16],
    pub signature: [u8; 64],
}

/// Why a signed request is refused: 401 `signature` or 401 `clock`.
#[derive(Debug, PartialEq)]
pub enum Bad {
    Signature,
    Clock,
}

/// The text a signature covers: the six lines joined by `\n`.
pub fn text(
    method: &str,
    target: &str,
    time: u64,
    nonce: &[u8; 16],
    body_sha256_hex: &str,
) -> Vec<u8> {
    let _ = (method, target, time, nonce, body_sha256_hex);
    unimplemented!("stream G")
}

/// The four headers that sign a request with `key` at `time`, with a fresh random nonce: `(name, value)` pairs in
/// the order `KEY`, `TIME`, `NONCE`, `SIGNATURE`.
pub fn headers(
    key: &SignKey,
    method: &str,
    target: &str,
    time: u64,
    body: &[u8],
) -> Result<Vec<(&'static str, String)>, String> {
    let _ = (key, method, target, time, body);
    unimplemented!("stream G")
}

/// Reads the four headers through `header`: `Ok(None)` when the request carries none of them, `Err(Signature)` when
/// it carries only some, or one that is not in its hex form.
pub fn read(header: &dyn Fn(&str) -> Option<String>) -> Result<Option<Signed>, Bad> {
    let _ = header;
    unimplemented!("stream G")
}

/// Checks `signed` for a request at the relay's clock `now`: the time window first, then the signature.
pub fn verify(
    signed: &Signed,
    method: &str,
    target: &str,
    body_sha256_hex: &str,
    now: u64,
) -> Result<(), Bad> {
    let _ = (signed, method, target, body_sha256_hex, now);
    unimplemented!("stream G")
}
