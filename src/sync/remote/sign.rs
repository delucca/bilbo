//! The request signature both ends of a relay share: Ed25519 over `bilbo-relay-1`, the method, the target, the time,
//! the nonce and the body's SHA-256, carried in four hex headers.

use crate::identity::keys::{self, SignKey};
use crate::shared::hash;

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
    format!(
        "{DOMAIN}\n{method}\n{target}\n{time}\n{}\n{body_sha256_hex}",
        keys::hex(nonce)
    )
    .into_bytes()
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
    let nonce = keys::random::<16>()?;
    let signature = key.sign(&text(method, target, time, &nonce, &hash::sha256_hex(body)));
    Ok(vec![
        (KEY, keys::hex(&key.public())),
        (TIME, time.to_string()),
        (NONCE, keys::hex(&nonce)),
        (SIGNATURE, keys::hex(&signature)),
    ])
}

/// Reads the four headers through `header`: `Ok(None)` when the request carries none of them, `Err(Signature)` when
/// it carries only some, or one that is not in its hex form.
pub fn read(header: &dyn Fn(&str) -> Option<String>) -> Result<Option<Signed>, Bad> {
    let (key, time, nonce, signature) =
        (header(KEY), header(TIME), header(NONCE), header(SIGNATURE));
    let (Some(key), Some(time), Some(nonce), Some(signature)) = (&key, &time, &nonce, &signature)
    else {
        return if key.is_none() && time.is_none() && nonce.is_none() && signature.is_none() {
            Ok(None)
        } else {
            Err(Bad::Signature)
        };
    };
    Ok(Some(Signed {
        key: keys::unhex(key).ok_or(Bad::Signature)?,
        time: decimal(time).ok_or(Bad::Signature)?,
        nonce: keys::unhex(nonce).ok_or(Bad::Signature)?,
        signature: keys::unhex(signature).ok_or(Bad::Signature)?,
    }))
}

/// Plain decimal digits with no sign and no leading zero.
fn decimal(text: &str) -> Option<u64> {
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    text.parse().ok()
}

/// Checks `signed` for a request at the relay's clock `now`: the time window first, then the signature.
pub fn verify(
    signed: &Signed,
    method: &str,
    target: &str,
    body_sha256_hex: &str,
    now: u64,
) -> Result<(), Bad> {
    if signed.time.abs_diff(now) > WINDOW {
        return Err(Bad::Clock);
    }
    let message = text(method, target, signed.time, &signed.nonce, body_sha256_hex);
    if keys::verify(&signed.key, &message, &signed.signature) {
        Ok(())
    } else {
        Err(Bad::Signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn key() -> SignKey {
        SignKey::from_seed(&[7; 32])
    }

    fn signed_with(
        method: &str,
        target: &str,
        time: u64,
        body: &[u8],
    ) -> Vec<(&'static str, String)> {
        headers(&key(), method, target, time, body).unwrap()
    }

    fn read_from(list: &[(&'static str, String)]) -> Result<Option<Signed>, Bad> {
        read(&|name| {
            list.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        })
    }

    fn set(list: &mut [(&'static str, String)], name: &str, value: &str) {
        list.iter_mut().find(|(n, _)| *n == name).unwrap().1 = value.to_string();
    }

    #[test]
    fn text_is_six_lines() {
        let text = text("PUT", "/v1/x", 5, &[0xab; 16], "cd");
        let expect = format!("bilbo-relay-1\nPUT\n/v1/x\n5\n{}\ncd", "ab".repeat(16));
        assert_eq!(String::from_utf8(text).unwrap(), expect);
    }

    #[test]
    fn a_round_trip_verifies() {
        let list = signed_with("PUT", "/v1/scopes/", NOW, b"body");
        assert_eq!(
            list.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            [KEY, TIME, NONCE, SIGNATURE]
        );
        assert_eq!(list[0].1.len(), 64);
        assert_eq!(list[2].1.len(), 32);
        assert_eq!(list[3].1.len(), 128);
        let signed = read_from(&list).unwrap().unwrap();
        assert_eq!(signed.key, key().public());
        let digest = hash::sha256_hex(b"body");
        assert_eq!(verify(&signed, "PUT", "/v1/scopes/", &digest, NOW), Ok(()));
    }

    #[test]
    fn each_signature_has_a_fresh_nonce() {
        let a = signed_with("GET", "/v1/scopes/", NOW, b"");
        let b = signed_with("GET", "/v1/scopes/", NOW, b"");
        assert_ne!(a[2].1, b[2].1);
    }

    #[test]
    fn a_changed_body_target_or_method_is_a_bad_signature() {
        let signed = read_from(&signed_with("PUT", "/v1/a", NOW, b"body"))
            .unwrap()
            .unwrap();
        let digest = hash::sha256_hex(b"body");
        assert_eq!(verify(&signed, "PUT", "/v1/a", &digest, NOW), Ok(()));
        let other = hash::sha256_hex(b"bodz");
        assert_eq!(
            verify(&signed, "PUT", "/v1/a", &other, NOW),
            Err(Bad::Signature)
        );
        assert_eq!(
            verify(&signed, "PUT", "/v1/b", &digest, NOW),
            Err(Bad::Signature)
        );
        assert_eq!(
            verify(&signed, "GET", "/v1/a", &digest, NOW),
            Err(Bad::Signature)
        );
        let queried = read_from(&signed_with("GET", "/v1/a?after=1", NOW, b""))
            .unwrap()
            .unwrap();
        let empty = hash::sha256_hex(b"");
        assert_eq!(
            verify(&queried, "GET", "/v1/a?after=1", &empty, NOW),
            Ok(())
        );
        assert_eq!(
            verify(&queried, "GET", "/v1/a?after=2", &empty, NOW),
            Err(Bad::Signature)
        );
        assert_eq!(
            verify(&queried, "GET", "/v1/a", &empty, NOW),
            Err(Bad::Signature)
        );
    }

    #[test]
    fn another_key_is_a_bad_signature() {
        let mut signed = read_from(&signed_with("GET", "/v1/a", NOW, b""))
            .unwrap()
            .unwrap();
        signed.key = SignKey::from_seed(&[8; 32]).public();
        assert_eq!(
            verify(&signed, "GET", "/v1/a", &hash::sha256_hex(b""), NOW),
            Err(Bad::Signature)
        );
    }

    #[test]
    fn the_window_is_300_seconds_either_way() {
        let digest = hash::sha256_hex(b"");
        for (time, expect) in [
            (NOW - 300, Ok(())),
            (NOW + 300, Ok(())),
            (NOW - 301, Err(Bad::Clock)),
            (NOW + 301, Err(Bad::Clock)),
        ] {
            let signed = read_from(&signed_with("GET", "/v1/a", time, b""))
                .unwrap()
                .unwrap();
            assert_eq!(
                verify(&signed, "GET", "/v1/a", &digest, NOW),
                expect,
                "{time}"
            );
        }
    }

    #[test]
    fn the_window_is_checked_before_the_signature() {
        let mut signed = read_from(&signed_with("GET", "/v1/a", NOW - 301, b""))
            .unwrap()
            .unwrap();
        signed.signature = [0; 64];
        assert_eq!(
            verify(&signed, "GET", "/v1/a", &hash::sha256_hex(b""), NOW),
            Err(Bad::Clock)
        );
    }

    #[test]
    fn no_headers_is_none() {
        assert_eq!(read(&|_| None), Ok(None));
    }

    #[test]
    fn a_missing_header_is_a_bad_signature() {
        let full = signed_with("GET", "/v1/a", NOW, b"");
        for skip in [KEY, TIME, NONCE, SIGNATURE] {
            let list: Vec<_> = full.iter().filter(|(n, _)| *n != skip).cloned().collect();
            assert_eq!(read_from(&list), Err(Bad::Signature), "{skip}");
        }
    }

    #[test]
    fn a_malformed_field_is_a_bad_signature() {
        let full = signed_with("GET", "/v1/a", NOW, b"");
        let upper = full[0].1.to_uppercase();
        let cases: Vec<(&str, String)> = vec![
            (KEY, upper),
            (KEY, full[0].1[2..].to_string()),
            (KEY, format!("{}zz", &full[0].1[2..])),
            (NONCE, full[2].1[2..].to_string()),
            (NONCE, format!("{}00", full[2].1)),
            (SIGNATURE, full[3].1[2..].to_string()),
            (SIGNATURE, format!("{}0g", &full[3].1[2..])),
            (TIME, String::new()),
            (TIME, "+5".into()),
            (TIME, "-5".into()),
            (TIME, "05".into()),
            (TIME, "0x5".into()),
            (TIME, " 5".into()),
            (TIME, "5 ".into()),
            (TIME, "1.5".into()),
            (TIME, "99999999999999999999999".into()),
        ];
        for (name, value) in cases {
            let mut list = full.clone();
            set(&mut list, name, &value);
            assert_eq!(read_from(&list), Err(Bad::Signature), "{name}={value}");
        }
    }

    #[test]
    fn a_time_of_zero_is_plain_decimal() {
        let mut list = signed_with("GET", "/v1/a", NOW, b"");
        set(&mut list, TIME, "0");
        assert_eq!(read_from(&list).unwrap().unwrap().time, 0);
    }
}
