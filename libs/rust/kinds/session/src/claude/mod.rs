pub mod read;

use std::path::{Path, PathBuf};

use crate::dialect::{Dialect, Evidence};
use crate::json::Value;

/// Skipped outside `subagents/`: memories, workflow definitions and tool outputs.
/// `subagents/workflows/wf_*/` holds workflow runs' subagents and is kept.
pub const SKIPPED: [&str; 3] = ["memory", "workflows", "tool-results"];

pub fn is_subagents(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "subagents")
}

/// A string `type` is Claude Code's (Codex, with an object `payload`, is told apart first).
/// Cursor's entries have a user or assistant `role` and no `type`, or a null one. Only an entry
/// the reader renders something from is sure: any other may sit before another dialect's lines.
pub fn dialect(line: &Value) -> Option<Evidence> {
    let dialect = match (line.get("type"), line.get("role")) {
        (Some(Value::Str(_)), _) => Dialect::ClaudeCode,
        (None | Some(Value::Null), Some(Value::Str(role)))
            if matches!(role.as_str(), "user" | "assistant") =>
        {
            Dialect::Cursor
        }
        _ => return None,
    };
    Some(if read::renders(line) {
        Evidence::Sure(dialect)
    } else {
        Evidence::Maybe(dialect)
    })
}

/// `<folder>/<uuid>.jsonl`, by the nearest `subagents` folder whose parent is a UUID.
pub fn session_of(subagent: &Path) -> Option<PathBuf> {
    let name = subagent.file_name()?.to_str()?;
    if !(name.starts_with("agent-") && name.ends_with(".jsonl")) {
        return None;
    }
    subagent
        .ancestors()
        .skip(1)
        .filter(|folder| is_subagents(folder))
        .find_map(|subagents| {
            let session = subagents.parent()?;
            let uuid = session.file_name()?.to_str()?;
            crate::is_uuid(uuid.as_bytes()).then(|| session.with_file_name(format!("{uuid}.jsonl")))
        })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::session_of;

    const UUID: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";

    #[test]
    fn a_subagent_belongs_to_the_session_named_by_its_folder() {
        let subagent = format!("/p/proj/{UUID}/subagents/agent-a1.jsonl");
        assert_eq!(
            session_of(Path::new(&subagent)),
            Some(PathBuf::from(format!("/p/proj/{UUID}.jsonl")))
        );
        let workflow = format!("/p/proj/{UUID}/subagents/workflows/wf_1/agent-a2.jsonl");
        assert_eq!(
            session_of(Path::new(&workflow)),
            Some(PathBuf::from(format!("/p/proj/{UUID}.jsonl"))),
            "a workflow run's subagent belongs to the session too"
        );
        for other in [
            format!("/p/proj/{UUID}/subagents/agent-a1.meta.json"),
            format!("/p/proj/{UUID}/subagents/other.jsonl"),
            format!("/p/proj/{UUID}/subagents/workflows/wf_1/journal.jsonl"),
            format!("/p/proj/{UUID}/elsewhere/agent-a1.jsonl"),
            "/p/proj/not-a-uuid/subagents/agent-a1.jsonl".to_owned(),
            format!("/p/proj/{UUID}.jsonl"),
        ] {
            assert_eq!(session_of(Path::new(&other)), None, "{other}");
        }
    }

    #[test]
    fn a_subagent_below_a_nested_subagents_folder_belongs_to_the_uuid_one() {
        let nested = format!("/p/proj/{UUID}/subagents/workflows/wf_1/subagents/agent-x.jsonl");
        assert_eq!(
            session_of(Path::new(&nested)),
            Some(PathBuf::from(format!("/p/proj/{UUID}.jsonl")))
        );
        let none = "/p/proj/not-a-uuid/subagents/w/subagents/agent-x.jsonl";
        assert_eq!(session_of(Path::new(none)), None);
    }
}
