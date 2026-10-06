//! The relay's start-up walk: every scope in the data folder read as a `file://` transport, its chain verified as a
//! device verifies it, its seqs and owner checked, and its state rebuilt. Nothing on disk is trusted or changed.

use std::collections::BTreeMap;

use super::admit::Held;
use super::store::Data;

/// The scopes under `<data>/scopes/`, each `Valid`, `NotAdmitted` or `Invalid`, with its byte total, latest version
/// and each device's highest seq. Each scope that is not valid gets one line on `log`.
pub fn walk(
    data: &Data,
    owners: &[String],
    log: &(dyn Fn(&str) + Sync),
) -> Result<BTreeMap<String, Held>, String> {
    let _ = (data, owners, log);
    unimplemented!("stream L")
}
