use crate::json::{self, Value};
use crate::{claude, codex};

/// Sniffed on each read and never stored, so it cannot go stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    ClaudeCode,
    /// Claude-shaped entries that carry `role` and no `type`.
    Cursor,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    Sure(Dialect),
    Maybe(Dialect),
}

/// Lines are read as the readers read them, so what a reader renders is never refused here.
pub fn sniff<'r>(records: impl IntoIterator<Item = &'r [u8]>) -> Option<Dialect> {
    let mut maybe = None;
    for record in records {
        match line(record) {
            Some(Evidence::Sure(dialect)) => return Some(dialect),
            Some(Evidence::Maybe(dialect)) => maybe = maybe.or(Some(dialect)),
            None => {}
        }
    }
    maybe
}

fn line(record: &[u8]) -> Option<Evidence> {
    let value = json::parse(record).filter(Value::is_dict)?;
    codex::dialect(&value).or_else(|| claude::dialect(&value))
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
        let mixed = [&b"{\"a\":1}\n"[..], b"garbage\n", CURSOR, CODEX].concat();
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

    #[test]
    fn a_line_of_an_unknown_type_does_not_hide_the_dialect_of_the_lines_after_it() {
        let prelude = b"{\"type\":\"garbage\"}\n";
        assert_eq!(
            of(prelude),
            Some(Dialect::ClaudeCode),
            "alone it is Claude Code's"
        );
        assert_eq!(of(&[prelude, CODEX].concat()), Some(Dialect::Codex));
        assert_eq!(of(&[prelude, CURSOR].concat()), Some(Dialect::Cursor));
        assert_eq!(of(&[prelude, CLAUDE].concat()), Some(Dialect::ClaudeCode));
        let user = b"{\"type\":\"user\",\"message\":{\"content\":\"x\"}}\n";
        assert_eq!(
            of(&[user, CODEX].concat()),
            Some(Dialect::ClaudeCode),
            "a user entry is sure"
        );
    }

    #[test]
    fn a_line_its_reader_renders_nothing_from_is_never_sure() {
        for (weak, alone) in [
            (&b"{\"role\":\"user\"}\n"[..], Some(Dialect::Cursor)),
            (b"{\"type\":null,\"role\":\"garbage\"}\n", None),
            (b"{\"role\":\"system\"}\n", None),
            (
                b"{\"type\":\"user\",\"n\":NaN}\n",
                Some(Dialect::ClaudeCode),
            ),
            (
                b"{\"type\":\"user\",\"isMeta\":true,\"message\":{\"content\":\"x\"}}\n",
                Some(Dialect::ClaudeCode),
            ),
            (
                b"{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"thinking\"}]}}\n",
                Some(Dialect::ClaudeCode),
            ),
        ] {
            let shown = String::from_utf8_lossy(weak);
            assert_eq!(of(weak), alone, "{shown}");
            assert_eq!(of(&[weak, CODEX].concat()), Some(Dialect::Codex), "{shown}");
        }
    }

    #[test]
    fn a_line_is_read_as_the_readers_read_it() {
        assert_eq!(
            of(b"{\"type\":null,\"role\":\"user\",\"message\":{\"content\":\"hello\"}}\n"),
            Some(Dialect::Cursor),
            "a null type falls back to the role"
        );
        assert_eq!(
            of(b"{\"type\":\"user\",\"n\":NaN,\"message\":{\"content\":\"hello\"}}\n"),
            Some(Dialect::ClaudeCode),
            "Python reads NaN"
        );
        assert_eq!(
            of(b"{\"type\":\"response_item\",\"n\":Infinity,\"payload\":{}}\n"),
            Some(Dialect::Codex)
        );
    }
}
