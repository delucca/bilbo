use serde_json::Value;

use crate::{claude, codex};

/// Sniffed on each read and never stored, so it cannot go stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    ClaudeCode,
    /// Claude-shaped entries that carry `role` and no `type`.
    Cursor,
    Codex,
}

/// Only `is_none()` is read today; the dialect itself is the seam for a third harness (AGY).
pub fn sniff<'r>(records: impl IntoIterator<Item = &'r [u8]>) -> Option<Dialect> {
    records.into_iter().find_map(line)
}

fn line(record: &[u8]) -> Option<Dialect> {
    let value: Value = serde_json::from_slice(record).ok()?;
    let object = value.as_object()?;
    codex::dialect(object).or_else(|| claude::dialect(object))
}

#[cfg(test)]
mod tests {
    use super::{Dialect, sniff};

    const CLAUDE: &[u8] = include_bytes!("../fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl");
    const SUBAGENT: &[u8] = include_bytes!("../fixtures/agent-ad63c2b3c9e7a4b3d.jsonl");
    const CODEX: &[u8] = include_bytes!(
        "../fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
    );
    const CURSOR: &[u8] = include_bytes!("../fixtures/cursor-session.jsonl");
    const NONE: &[u8] = include_bytes!("../fixtures/not-a-session.jsonl");

    fn of(bytes: &[u8]) -> Option<Dialect> {
        sniff(bytes.split_inclusive(|&byte| byte == b'\n'))
    }

    #[test]
    fn each_harness_is_told_apart_by_its_lines() {
        assert_eq!(of(CLAUDE), Some(Dialect::ClaudeCode));
        assert_eq!(of(SUBAGENT), Some(Dialect::ClaudeCode));
        assert_eq!(of(CODEX), Some(Dialect::Codex));
        assert_eq!(of(CURSOR), Some(Dialect::Cursor));
    }

    #[test]
    fn a_file_of_other_json_lines_is_no_dialect() {
        assert_eq!(of(NONE), None);
        assert_eq!(of(b""), None);
        assert_eq!(of(b"not json\n[1,2]\n\"text\"\n"), None);
    }

    #[test]
    fn the_first_line_that_has_a_dialect_decides() {
        let mixed = [
            &b"{\"a\":1}\n"[..],
            b"garbage\n",
            b"{\"role\":\"user\"}\n",
            CODEX,
        ]
        .concat();
        assert_eq!(of(&mixed), Some(Dialect::Cursor));
        assert_eq!(
            of(b"{\"type\":\"summary\",\"summary\":\"s\"}\n"),
            Some(Dialect::ClaudeCode)
        );
        assert_eq!(
            of(b"{\"type\":\"event_msg\",\"payload\":{}}\n"),
            Some(Dialect::Codex)
        );
        assert_eq!(of(b"{\"type\":3}\n{\"role\":4}\n"), None);
    }
}
