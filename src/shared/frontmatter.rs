//! The frontmatter values every store file shares: the `<key>: <value>` line, ULID ids and `created` times. Each
//! kind of file splits its own block (`note::read`, `source::split_front`).

use std::io::Read;

pub const CREATED_FORMAT: &str = "%Y-%m-%dT%H:%M%:z";

const ULID_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub fn mint_ulid() -> std::io::Result<String> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let mut random = [0u8; 10];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    Ok(encode_ulid(ms, random))
}

fn encode_ulid(ms: u64, random: [u8; 10]) -> String {
    let mut bytes = [0u8; 16];
    bytes[6..].copy_from_slice(&random);
    let v = ((ms & ((1 << 48) - 1)) as u128) << 80 | u128::from_be_bytes(bytes);
    (0..26)
        .map(|i| ULID_ALPHABET[((v >> (125 - 5 * i)) & 31) as usize] as char)
        .collect()
}

pub fn is_ulid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 26 && (b'0'..=b'7').contains(&b[0]) && b.iter().all(|c| ULID_ALPHABET.contains(c))
}

pub fn now_created() -> String {
    jiff::Zoned::now().strftime(CREATED_FORMAT).to_string()
}

pub fn is_created(s: &str) -> bool {
    const SHAPE: &[u8; 22] = b"dddd-dd-ddTdd:dd+dd:dd";
    let b = s.as_bytes();
    b.len() == 22
        && SHAPE.iter().zip(b).all(|(p, c)| match p {
            b'd' => c.is_ascii_digit(),
            b'+' => *c == b'+' || *c == b'-',
            _ => p == c,
        })
        && !s.ends_with("-00:00")
        && jiff::fmt::strtime::parse(CREATED_FORMAT, s)
            .and_then(|t| t.to_datetime())
            .is_ok()
}

pub fn bad_id(value: &str) -> String {
    format!(
        "id: '{value}' is not a canonical ULID: 26 characters of 0-9 and A-Z without I, L, O, U, the first 0-7"
    )
}

pub fn bad_created(value: &str) -> String {
    format!("created: '{value}' is not YYYY-MM-DDTHH:MM±HH:MM, a real local time to the minute")
}

pub fn split_key(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once(':')?;
    let valid = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    valid.then_some((key, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01M3YJ7R6HK6NQ30DCDB1P4DYB";
    const CREATED: &str = "2026-10-02T14:23-03:00";

    #[test]
    fn canonical_ulid_is_valid() {
        assert!(is_ulid(ID));
        assert!(is_ulid("7ZZZZZZZZZZZZZZZZZZZZZZZZZ"));
    }

    #[test]
    fn ulid_vectors() {
        assert_eq!(encode_ulid(0, [0; 10]), "00000000000000000000000000");
        assert_eq!(
            encode_ulid((1 << 48) - 1, [0xff; 10]),
            "7ZZZZZZZZZZZZZZZZZZZZZZZZZ"
        );
    }

    #[test]
    fn minted_ulids_are_canonical_and_distinct() {
        let minted: std::collections::HashSet<String> =
            (0..200).map(|_| mint_ulid().unwrap()).collect();
        assert_eq!(minted.len(), 200);
        assert!(minted.iter().all(|u| is_ulid(u)));
    }

    #[test]
    fn valid_created() {
        for s in [
            CREATED,
            "2026-10-02T14:23+00:00",
            "2024-02-29T10:00+05:30",
            "2026-10-02T14:23+05:45",
        ] {
            assert!(is_created(s), "{s}");
        }
    }

    #[test]
    fn impossible_date_is_invalid() {
        for s in [
            "2026-02-30T10:00-03:00",
            "2025-02-29T10:00-03:00",
            "2026-10-02T24:00-03:00",
        ] {
            assert!(!is_created(s), "{s}");
        }
    }

    #[test]
    fn now_created_is_valid() {
        let now = now_created();
        assert!(is_created(&now), "{now}");
    }
}
