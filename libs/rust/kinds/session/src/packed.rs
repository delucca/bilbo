use std::io::Read;
use std::path::{Path, PathBuf};

use bilbo_kinds::kind::Refusal;
use bilbo_kinds::remember::RECORD_HELD;
use zstd::stream::read::Decoder;

pub const SUFFIX: &str = ".zst";

pub fn plain(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(SUFFIX)
        .filter(|stem| !stem.is_empty())
        .map(|stem| path.with_file_name(stem))
}

pub fn beside(plain: &Path) -> PathBuf {
    let mut packed = plain.as_os_str().to_owned();
    packed.push(SUFFIX);
    PathBuf::from(packed)
}

pub fn sibling(path: &Path) -> PathBuf {
    plain(path).unwrap_or_else(|| beside(path))
}

pub fn bound(packed: usize, floor: u64, ratio: u64) -> u64 {
    let packed = u64::try_from(packed).unwrap_or(u64::MAX);
    floor.max(ratio.saturating_mul(packed))
}

/// A packer that knows its input's size asks for a window up to the next power of two above it, so
/// a file that passes the count must not fail on its window; zstd's own tools refuse more than 128 MiB.
fn window_log(limit: u64) -> u32 {
    (64 - limit.saturating_sub(1).leading_zeros()).clamp(23, 27)
}

pub fn unpack(bytes: &[u8], limit: u64) -> Result<Vec<u8>, Refusal> {
    let mut unpacked = Vec::new();
    let mut decoder =
        Decoder::with_buffer(bytes).map_err(|error| Refusal(format!("cannot unpack: {error}")))?;
    decoder
        .window_log_max(window_log(limit))
        .map_err(|error| Refusal(format!("cannot unpack: {error}")))?;
    decoder
        .take(limit.saturating_add(1))
        .read_to_end(&mut unpacked)
        .map_err(|error| Refusal(format!("cannot unpack: {error}")))?;
    if held(&unpacked) > limit {
        return Err(Refusal(format!(
            "would hold more than {limit} bytes unpacked"
        )));
    }
    Ok(unpacked)
}

/// A line is a record, and `remember` holds `RECORD_HELD` for each beside its bytes.
fn held(unpacked: &[u8]) -> u64 {
    let lines = unpacked.iter().filter(|&&byte| byte == b'\n').count();
    let lines = u64::try_from(lines).unwrap_or(u64::MAX);
    let bytes = u64::try_from(unpacked.len()).unwrap_or(u64::MAX);
    bytes.saturating_add(lines.saturating_mul(RECORD_HELD))
}
