use std::io::Read;
use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Files, Node};
use bilbo_kinds::remember::{self, Request};
use bilbo_store::entries::StoreError;
use bilbo_store::segments::{Held, Options, Rebuild, Sealing, Store};
use serde_json::Value;

use super::disk::Disk;
use super::messages::{refusals, warning};
use super::registry;
use crate::console::env::Env;
use crate::console::message::{Message, sanitize};
use crate::console::outcome::{Failure, Outcome};
use crate::console::settings::Settings;

/// Keeps the transcript the harness hook names on stdin. Never seals, rebuilds or reruns follows:
/// those wait for the next run. Exits 0 whatever happens, since a failing hook can block the
/// harness; errors are one stderr line.
pub fn run(env: &mut Env, settings: Result<Settings, Failure>) -> Outcome {
    let diagnostics = match keep(env, settings) {
        Ok(diagnostics) => diagnostics,
        Err(line) => vec![line],
    };
    Outcome::new("").with_diagnostics(diagnostics)
}

fn keep(env: &mut Env, settings: Result<Settings, Failure>) -> Result<Vec<Message>, Message> {
    let settings = settings.map_err(|failure| failure.message())?;
    let mut input = Vec::new();
    env.stdin
        .read_to_end(&mut input)
        .map_err(|error| warning(&format!("cannot read the hook input: {error}")))?;
    let target = transcript(&input).map_err(|reason| warning(&reason))?;
    regular(&target, &Disk).map_err(|reason| warning(&reason))?;
    let kinds = registry::kinds();
    let request = Request {
        kind: bilbo_kinds::follows::kind_of(&settings.follow.value, &target, &Disk),
        target,
    };
    let plan =
        remember::plan(&request, &kinds, &Disk).map_err(|error| warning(&error.to_string()))?;
    if plan.files.is_empty() {
        return Err(warning(&format!(
            "nothing to keep: no kind claims {}",
            plan.target.display()
        )));
    }
    let layout = settings.layout();
    Store::check(&layout).map_err(|error| Failure::from(&error).message())?;
    let options = Options {
        held: Held::Flag,
        sealing: Sealing::Never,
        rebuild: Rebuild::CatchUpOnly,
        ..settings.store.options()
    };
    let mut store = match Store::open(&layout, options, &mut |_| {}) {
        Ok(store) => store,
        Err(StoreError::Busy) => return Ok(Vec::new()),
        Err(StoreError::Unrebuilt) => {
            return Err(warning(
                "kept nothing: the catalog must be rebuilt first, which the next bilbo remember \
                 without --hook does",
            ));
        }
        Err(error) => return Err(Failure::from(&error).message()),
    };
    let kept = remember::keep(&plan, &Disk, &mut store);
    store.release();
    let output = kept.map_err(|error| Message::error(sanitize(&error.to_string())))?;
    Ok(refusals(std::slice::from_ref(&output)))
}

fn transcript(input: &[u8]) -> Result<PathBuf, String> {
    let value: Value = serde_json::from_slice(input)
        .ok()
        .filter(Value::is_object)
        .ok_or("the hook input is not a JSON object")?;
    let path = value
        .get("transcript_path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or("the hook input has no transcript_path")?;
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "the hook input's transcript_path is not an absolute path: {}",
            path.display()
        ))
    }
}

/// A folder would be walked, a device or pipe read without end.
fn regular(transcript: &Path, files: &dyn Files) -> Result<(), String> {
    let unreadable =
        |error: std::io::Error| format!("cannot read {}: {error}", transcript.display());
    match files
        .canonical(transcript)
        .and_then(|path| files.node(&path))
        .map_err(unreadable)?
    {
        Node::File => Ok(()),
        Node::Folder | Node::Other => Err(format!(
            "the hook input's transcript_path is not a regular file: {}",
            transcript.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use bilbo_kinds::files::Memory;

    use super::{regular, transcript};

    const CLAUDE: &str = include_str!("../../tests/fixtures/hook/claude-code-stop.json");
    const CODEX: &str = include_str!("../../tests/fixtures/hook/codex-stop.json");

    #[test]
    fn a_claude_code_or_codex_stop_payload_names_its_transcript() {
        assert_eq!(
            transcript(CLAUDE.as_bytes()),
            Ok(PathBuf::from(
                "/home/me/.claude/projects/-home-me-src-garden/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"
            ))
        );
        assert_eq!(
            transcript(CODEX.as_bytes()),
            Ok(PathBuf::from(
                "/home/me/.codex/sessions/2026/11/06/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
            ))
        );
    }

    #[test]
    fn fields_the_payload_adds_are_left_alone() {
        let extra = r#"{"transcript_path":"/t/a.jsonl","new_field":{"x":[1,2]},"cwd":3}"#;
        assert_eq!(
            transcript(extra.as_bytes()),
            Ok(PathBuf::from("/t/a.jsonl"))
        );
    }

    #[test]
    fn a_payload_without_a_usable_transcript_path_says_why() {
        for (input, reason) in [
            ("", "the hook input is not a JSON object"),
            ("garbage\n", "the hook input is not a JSON object"),
            ("[1,2]", "the hook input is not a JSON object"),
            (
                r#"{"session_id":"s"}"#,
                "the hook input has no transcript_path",
            ),
            (
                r#"{"transcript_path":null}"#,
                "the hook input has no transcript_path",
            ),
            (
                r#"{"transcript_path":""}"#,
                "the hook input has no transcript_path",
            ),
            (
                r#"{"transcript_path":7}"#,
                "the hook input has no transcript_path",
            ),
            (
                r#"{"transcript_path":"~/.claude/x.jsonl"}"#,
                "the hook input's transcript_path is not an absolute path: ~/.claude/x.jsonl",
            ),
        ] {
            assert_eq!(
                transcript(input.as_bytes()),
                Err(reason.to_owned()),
                "{input:?}"
            );
        }
    }

    #[test]
    fn only_a_regular_file_is_a_transcript() {
        let files = Memory::new()
            .file("/h/s.jsonl", "{}\n")
            .folder("/h/projects")
            .other("/h/pipe");
        assert_eq!(regular(Path::new("/h/s.jsonl"), &files), Ok(()));
        for path in ["/h/projects", "/h/pipe"] {
            assert_eq!(
                regular(Path::new(path), &files),
                Err(format!(
                    "the hook input's transcript_path is not a regular file: {path}"
                ))
            );
        }
        assert_eq!(
            regular(Path::new("/h/gone.jsonl"), &files),
            Err("cannot read /h/gone.jsonl: entity not found".to_owned())
        );
    }
}
