//! The device that shows the code: claims a mailbox, waits for the answer, asks the user, and enrolls the device that
//! answered.

use super::Cx;
use crate::Failure;

/// Shows a code for `scopes` (every syncing scope when empty) over `via` (the one URL they share when `None`).
pub fn run(cx: &mut Cx, scopes: &[String], via: Option<&str>) -> Result<(), Failure> {
    let _ = (cx, scopes, via);
    Err(Failure::Refused("bilbo pair is not built yet".into()))
}
