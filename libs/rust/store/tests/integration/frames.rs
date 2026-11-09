//! A reader of the documented frame layout that shares no code with the store.

use std::fs;
use std::path::Path;

pub type Summary = (u8, [u8; 16], u32, u64, u64);

pub struct Span {
    pub offset: usize,
    pub total: usize,
    pub summary: Summary,
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"))
}

pub fn spans(path: &Path) -> Vec<Span> {
    let bytes = fs::read(path).expect("read segment");
    assert_eq!(&bytes[..8], b"BILBOSEG");
    assert_eq!(u32_at(&bytes, 8), 1);
    let mut found = Vec::new();
    let mut at = 12;
    while at < bytes.len() {
        let total = 4 + u32_at(&bytes, at) as usize;
        found.push(Span {
            offset: at,
            total,
            summary: (
                bytes[at + 16],
                bytes[at + 17..at + 33].try_into().expect("entry"),
                u32_at(&bytes, at + 33),
                u64_at(&bytes, at + 37),
                u64_at(&bytes, at + 45),
            ),
        });
        at += total;
    }
    assert_eq!(at, bytes.len(), "{} ends inside a frame", path.display());
    found
}

pub fn frames(path: &Path) -> Vec<Summary> {
    spans(path).into_iter().map(|span| span.summary).collect()
}

/// `frame` with another payload, its length and both checksums recomputed, so only the payload
/// is wrong.
pub fn forge(frame: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut forged = frame[..77].to_vec();
    forged[..4].copy_from_slice(
        &u32::try_from(73 + payload.len())
            .expect("length")
            .to_le_bytes(),
    );
    forged.extend_from_slice(payload);
    let mut header = xxhash_rust::xxh3::Xxh3::new();
    header.update(&forged[..4]);
    header.update(&forged[16..77]);
    forged[4..8].copy_from_slice(&(header.digest() as u32).to_le_bytes());
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    hasher.update(&forged[..8]);
    hasher.update(&forged[16..]);
    forged[8..16].copy_from_slice(&hasher.digest().to_le_bytes());
    forged
}

pub fn compress(plain: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(
        plain,
        bilbo_store::segments::Options::default()
            .compression
            .level(),
    )
    .expect("compress")
}

/// `path` with the frame at `span` replaced by `frame`.
pub fn replace(path: &Path, span: &Span, frame: &[u8]) {
    let bytes = fs::read(path).expect("read");
    let mut out = bytes[..span.offset].to_vec();
    out.extend_from_slice(frame);
    out.extend_from_slice(&bytes[span.offset + span.total..]);
    fs::write(path, out).expect("write");
}
