pub mod read;

use crate::dialect::{Dialect, Evidence};
use crate::json::Value;

/// A Codex rollout line has a string `type` and an object `payload`.
pub fn dialect(line: &Value) -> Option<Evidence> {
    match (line.get("type"), line.get("payload")) {
        (Some(Value::Str(_)), Some(Value::Dict(_))) => Some(Evidence::Sure(Dialect::Codex)),
        _ => None,
    }
}
