//! The device that joins: answers a code, then checks and writes what the showing device sends.

use super::Cx;
use crate::Failure;

/// Answers `code` on the transport at `via`, as `name` when this device has no keys yet.
pub fn run(cx: &mut Cx, code: &str, via: &str, name: Option<&str>) -> Result<(), Failure> {
    let _ = (cx, code, via, name);
    Err(Failure::Refused("bilbo pair is not built yet".into()))
}
