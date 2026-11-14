use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::time::{SystemTime, UNIX_EPOCH};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const LENGTH: usize = 26;
const SHORT_BITS: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(u128);

impl Id {
    /// 26 characters of the Crockford alphabet, uppercase only, the first 0-7.
    #[must_use]
    pub fn parse(text: &str) -> Option<Id> {
        let bytes = text.as_bytes();
        if bytes.len() != LENGTH || bytes[0] > b'7' {
            return None;
        }
        let mut value: u128 = 0;
        for &byte in bytes {
            let digit = ALPHABET.iter().position(|&c| c == byte)?;
            value = (value << 5) | digit as u128;
        }
        Some(Id(value))
    }

    /// 48 bits of Unix milliseconds, then 80 bits read from `/dev/urandom`.
    pub fn mint() -> io::Result<Id> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| io::Error::other("the clock is before 1970"))?
            .as_millis();
        let mut random = [0u8; 10];
        File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut random))
            .map_err(|error| {
                io::Error::new(error.kind(), format!("cannot read /dev/urandom: {error}"))
            })?;
        let random = random
            .iter()
            .fold(0u128, |acc, &b| (acc << 8) | u128::from(b));
        Ok(Id(((millis & 0xFFFF_FFFF_FFFF) << 80) | random))
    }

    /// `mint()`, or `floor` plus one when that is not greater than `floor`.
    /// A floor of the greatest id has no successor.
    pub fn mint_after(floor: Option<Id>) -> io::Result<Id> {
        let minted = Id::mint()?;
        match floor {
            Some(floor) if minted <= floor => floor
                .0
                .checked_add(1)
                .map(Id)
                .ok_or_else(|| io::Error::other("no id is greater than the greatest id")),
            _ => Ok(minted),
        }
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 16]) -> Id {
        Id(u128::from_be_bytes(bytes))
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }

    #[must_use]
    pub fn short(self) -> Short {
        Short((self.0 & ((1 << SHORT_BITS) - 1)) as u64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Short(u64);

impl Short {
    pub const LENGTH: usize = 10;

    /// 10 characters of the id alphabet, upper case only.
    #[must_use]
    pub fn parse(text: &str) -> Option<Short> {
        let bytes = text.as_bytes();
        if bytes.len() != Short::LENGTH {
            return None;
        }
        let mut value: u64 = 0;
        for &byte in bytes {
            let digit = ALPHABET.iter().position(|&c| c == byte)?;
            value = (value << 5) | digit as u64;
        }
        Some(Short(value))
    }

    #[must_use]
    pub fn ends(self, id: Id) -> bool {
        id.short() == self
    }

    /// The low 48 bits, the six bytes a catalog key ends with.
    #[must_use]
    pub(crate) fn tail(self) -> [u8; 6] {
        let bytes = self.0.to_be_bytes();
        [bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]
    }
}

fn written<const N: usize>(value: u128, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let mut text = [0u8; N];
    for (index, slot) in text.iter_mut().enumerate() {
        let shift = 5 * (N - 1 - index);
        *slot = ALPHABET[((value >> shift) & 31) as usize];
    }
    f.write_str(std::str::from_utf8(&text).map_err(|_| fmt::Error)?)
}

impl fmt::Display for Short {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        written::<{ Short::LENGTH }>(u128::from(self.0), f)
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        written::<LENGTH>(self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{Id, Short};

    #[test]
    fn a_short_id_is_the_end_of_its_ids_text() {
        let id = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
        assert_eq!(id.short().to_string(), "2T6W9YB0DE");
        let zero = Id::from_bytes([0; 16]);
        assert_eq!(zero.short().to_string(), "0000000000");
        let greatest = Id::from_bytes([0xFF; 16]);
        assert_eq!(greatest.short().to_string(), "ZZZZZZZZZZ");
    }

    #[test]
    fn a_short_id_is_ten_upper_case_characters_of_the_id_alphabet() {
        for text in ["2T6W9YB0DE", "0000000000", "ZZZZZZZZZZ"] {
            let short = Short::parse(text).expect(text);
            assert_eq!(short.to_string(), text);
        }
        let cases = [
            "2t6w9yb0de",
            "2T6W9YB0D",
            "2T6W9YB0DEE",
            "2T6W9YB0DI",
            "2T6W9YB0DL",
            "2T6W9YB0DO",
            "2T6W9YB0DU",
            "2T6W9YB0D\u{e9}",
            "",
        ];
        for case in cases {
            assert_eq!(Short::parse(case), None, "{case:?}");
        }
    }

    #[test]
    fn a_short_id_ends_the_ids_that_share_its_fifty_bits_and_no_other() {
        let base = u128::from_be_bytes(
            Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE")
                .expect("id")
                .to_bytes(),
        );
        let short = Id::from_bytes(base.to_be_bytes()).short();
        let id = |value: u128| Id::from_bytes(value.to_be_bytes());
        assert!(short.ends(id(base)));
        assert!(short.ends(id(base ^ (1 << 50))));
        assert!(short.ends(id(base ^ (1 << 120))));
        assert!(!short.ends(id(base ^ (1 << 49))));
        assert!(!short.ends(id(base ^ 1)));
    }

    #[test]
    fn a_ulid_reads_back_as_the_same_text_and_bytes() {
        let text = "01JC5Q3V7XKM4P8R2T6W9YB0DB";
        let id = Id::parse(text).expect("canonical");
        assert_eq!(id.to_string(), text);
        assert_eq!(Id::from_bytes(id.to_bytes()), id);
        assert_eq!(
            Id::parse("7ZZZZZZZZZZZZZZZZZZZZZZZZZ").map(Id::to_bytes),
            Some([0xFF; 16])
        );
        assert_eq!(
            Id::parse("00000000000000000000000000").map(Id::to_bytes),
            Some([0; 16])
        );
    }

    #[test]
    fn only_canonical_ulids_parse() {
        let cases = [
            "01jc5q3v7xkm4p8r2t6w9yb0db",
            "01JC5Q3V7XKM4P8R2T6W9YB0DI",
            "01JC5Q3V7XKM4P8R2T6W9YB0DL",
            "01JC5Q3V7XKM4P8R2T6W9YB0DO",
            "01JC5Q3V7XKM4P8R2T6W9YB0DU",
            "81JC5Q3V7XKM4P8R2T6W9YB0DE",
            "01JC5Q3V7XKM4P8R2T6W9YB0D",
            "01JC5Q3V7XKM4P8R2T6W9YB0DBB",
            "01JC5Q3V7XKM4P8R2T6W9YB0D\u{e9}",
            "",
        ];
        for case in cases {
            assert_eq!(Id::parse(case), None, "{case:?}");
        }
    }

    #[test]
    fn minted_ids_are_canonical_and_distinct() {
        let mut seen = HashSet::new();
        for _ in 0..200 {
            let id = Id::mint().expect("mint");
            assert_eq!(Id::parse(&id.to_string()), Some(id));
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn an_id_minted_after_another_is_greater_even_in_the_same_millisecond() {
        let minted = Id::mint().expect("mint");
        let ahead =
            Id::from_bytes((u128::from_be_bytes(minted.to_bytes()) + (1 << 79)).to_be_bytes());
        let next = Id::mint_after(Some(ahead)).expect("successor");
        assert!(next > ahead);
        assert!(Id::mint_after(None).is_ok());
    }

    #[test]
    fn the_greatest_id_has_no_successor() {
        let greatest = Id::from_bytes([0xFF; 16]);
        assert!(Id::mint_after(Some(greatest)).is_err());
    }

    #[test]
    fn a_minted_id_is_the_clock_and_80_random_bits() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis();
        let ids: Vec<Id> = (0..64).map(|_| Id::mint().expect("mint")).collect();
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis();
        let random = (1u128 << 80) - 1;
        let mut any = 0u128;
        let mut all = random;
        for id in &ids {
            let stamp = id.0 >> 80;
            assert!(
                (before..=after).contains(&stamp),
                "{stamp} not in {before}..={after}"
            );
            any |= id.0 & random;
            all &= id.0;
        }
        assert_eq!(any, random, "every random bit is set by some id");
        assert_ne!(all, random, "no id is all ones");
    }

    #[test]
    fn a_floor_below_the_clock_does_not_change_the_minted_id() {
        let floor = Id::from_bytes([0; 16]);
        let next = Id::mint_after(Some(floor)).expect("mint");
        assert!(next.0 >> 80 > 0, "{next}");
    }

    #[test]
    fn digits_parse_to_their_values() {
        assert_eq!(
            Id::parse("00000000000000000000000001").map(|id| id.0),
            Some(1)
        );
        assert_eq!(
            Id::parse("0000000000000000000000000Z").map(|id| id.0),
            Some(31)
        );
        assert_eq!(
            Id::parse("00000000000000000000000010").map(|id| id.0),
            Some(32)
        );
    }
}
