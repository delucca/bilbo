//! The recovery phrase: the BIP39 English list, its checksum and the 4-letter prefixes.

use crate::shared::hash;
use zeroize::Zeroizing;

const LIST: &str = include_str!("bip39-english.txt");

/// The words of a phrase.
pub const WORDS: usize = 12;

/// A phrase as indexes into the list, so no word is held as text.
pub type Phrase = Zeroizing<[u16; WORDS]>;

fn list() -> impl Iterator<Item = &'static str> {
    LIST.lines()
}

/// The word at `index` of the list.
pub fn word(index: u16) -> &'static str {
    list().nth(usize::from(index)).unwrap()
}

/// The 12 words of 128 bits: the bits and the first 4 bits of their SHA-256, in groups of 11.
pub fn encode(entropy: &[u8; 16]) -> Phrase {
    let mut bits = Zeroizing::new([0u8; 17]);
    bits[..16].copy_from_slice(entropy);
    bits[16] = hash::sha256(entropy)[0] & 0xf0;
    let mut phrase = Zeroizing::new([0u16; WORDS]);
    for (i, slot) in phrase.iter_mut().enumerate() {
        for bit in i * 11..(i + 1) * 11 {
            *slot = *slot << 1 | u16::from(bits[bit / 8] >> (7 - bit % 8) & 1);
        }
    }
    phrase
}

/// The 128 bits of 12 words, whose checksum must match.
pub fn decode(phrase: &[u16]) -> Result<Zeroizing<[u8; 16]>, String> {
    if phrase.len() != WORDS || phrase.iter().any(|&w| w >= 2048) {
        return Err("a phrase has 12 words of the list".to_string());
    }
    let mut bits = Zeroizing::new([0u8; 17]);
    for (i, &w) in phrase.iter().enumerate() {
        for k in 0..11 {
            let bit = i * 11 + k;
            bits[bit / 8] |= ((w >> (10 - k) & 1) as u8) << (7 - bit % 8);
        }
    }
    let mut entropy = Zeroizing::new([0u8; 16]);
    entropy.copy_from_slice(&bits[..16]);
    if hash::sha256(&*entropy)[0] & 0xf0 != bits[16] {
        return Err("the words do not make a valid phrase".to_string());
    }
    Ok(entropy)
}

/// The index of a word typed in full or by its first 4 letters, ignoring case and surrounding spaces.
pub fn lookup(typed: &str) -> Option<u16> {
    let typed = Zeroizing::new(typed.trim().to_ascii_lowercase());
    let index = list().position(|w| w == *typed).or_else(|| {
        if typed.len() == 4 {
            list().position(|w| w.starts_with(typed.as_str()))
        } else {
            None
        }
    })?;
    Some(index as u16)
}

/// For `Prompter::input`; the message names no word, and the caller adds the position.
pub fn check_word(typed: &str) -> Result<(), String> {
    lookup(typed)
        .map(|_| ())
        .ok_or_else(|| "not a word of the list".to_string())
}

/// Three distinct positions below `WORDS`, ascending, drawn from three random bytes.
pub fn positions(random: [u8; 3]) -> [usize; 3] {
    let mut left: Vec<usize> = (0..WORDS).collect();
    let mut picked = random.map(|byte| left.remove(usize::from(byte) % left.len()));
    picked.sort_unstable();
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const VECTORS: [(u8, &str); 4] = [
        (
            0x00,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        ),
        (
            0x7f,
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
        ),
        (
            0x80,
            "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
        ),
        (0xff, "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong"),
    ];

    fn typed(words: &str) -> Vec<u16> {
        words.split(' ').map(|w| lookup(w).unwrap()).collect()
    }

    #[test]
    fn the_list_is_pinned() {
        assert_eq!(list().count(), 2048);
        assert_eq!(
            hash::sha256_hex(LIST.as_bytes()),
            "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
        );
    }

    #[test]
    fn no_two_words_share_four_letters() {
        let prefixes: HashSet<&str> = list().map(|w| &w[..w.len().min(4)]).collect();
        assert_eq!(prefixes.len(), 2048);
    }

    #[test]
    fn the_bip39_vectors_both_ways() {
        for (byte, words) in VECTORS {
            let phrase = encode(&[byte; 16]);
            let spelled: Vec<&str> = phrase.iter().map(|&i| word(i)).collect();
            assert_eq!(spelled.join(" "), words);
            assert_eq!(*decode(&typed(words)).unwrap(), [byte; 16]);
        }
    }

    #[test]
    fn a_word_or_its_first_four_letters() {
        assert_eq!(lookup("abandon"), Some(0));
        assert_eq!(lookup("  ABAN "), Some(0));
        assert_eq!(lookup("Zoo"), Some(2047));
        assert_eq!(lookup("zoo "), Some(2047));
        assert_eq!(lookup("aba"), None);
        assert_eq!(lookup("abando"), None);
        assert_eq!(lookup("abandons"), None);
        assert_eq!(lookup(""), None);
    }

    #[test]
    fn an_unknown_word_names_nothing() {
        let err = check_word("qwertyuiop").unwrap_err();
        assert!(!err.contains("qwerty"));
        assert!(check_word("lega").is_ok());
    }

    #[test]
    fn a_failed_checksum_is_refused() {
        let mut words = typed(VECTORS[0].1);
        words[11] = lookup("abandon").unwrap();
        assert!(decode(&words).is_err());
        assert!(decode(&words[..11]).is_err());
    }

    #[test]
    fn positions_are_distinct_and_in_range() {
        for a in [0u8, 1, 11, 12, 255] {
            for b in [0u8, 5, 255] {
                for c in [0u8, 7, 254] {
                    let p = positions([a, b, c]);
                    assert!(p.iter().all(|&i| i < WORDS));
                    assert!(p[0] < p[1] && p[1] < p[2]);
                }
            }
        }
    }
}
