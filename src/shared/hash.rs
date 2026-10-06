use sha2::{Digest, Sha256};

/// The SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The SHA-256 of `bytes` in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A SHA-256 fed in pieces, for a body hashed as it streams.
#[derive(Default)]
pub struct Hasher(Sha256);

impl Hasher {
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// The digest in lowercase hex.
    pub fn hex(self) -> String {
        let digest: [u8; 32] = self.0.finalize().into();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fips_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pieces_hash_as_the_whole() {
        let mut hasher = Hasher::default();
        hasher.update(b"a");
        hasher.update(b"");
        hasher.update(b"bc");
        assert_eq!(hasher.hex(), sha256_hex(b"abc"));
        assert_eq!(Hasher::default().hex(), sha256_hex(b""));
    }
}
