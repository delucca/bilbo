pub mod read;

use crate::session::dialect::{Dialect, Evidence};
use crate::session::json::Value;

pub fn dialect(line: &Value) -> Option<Evidence> {
    match (line.get("type"), line.get("payload")) {
        (Some(Value::Str(_)), Some(Value::Dict(_))) => Some(Evidence::Sure(Dialect::Codex)),
        _ => None,
    }
}
