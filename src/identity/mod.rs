//! Who a device is and who owns it: the recovery phrase, the owner and device keys, the signed
//! manifests that say which devices may read a syncing scope, and the `device` verb.

pub mod ceremony;
pub mod device;
pub mod keys;
pub mod manifest;
pub mod phrase;
#[cfg(test)]
pub mod script;
