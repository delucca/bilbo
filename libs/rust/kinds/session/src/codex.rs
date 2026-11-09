use serde_json::{Map, Value};

use crate::dialect::Dialect;

/// A Codex rollout line has a string `type` and an object `payload`.
pub fn dialect(line: &Map<String, Value>) -> Option<Dialect> {
    match (line.get("type"), line.get("payload")) {
        (Some(Value::String(_)), Some(Value::Object(_))) => Some(Dialect::Codex),
        _ => None,
    }
}
