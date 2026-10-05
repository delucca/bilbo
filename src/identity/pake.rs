//! The pairing code and its exchange: parsing a code, SPAKE2 on both sides, the key schedule, the boxes, the
//! fingerprint and the three mailbox messages. No I/O.

use std::convert::Infallible;

use spake2::rand_core::{TryCryptoRng, TryRng};

use crate::identity::keys;

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
}
