mod claude;
mod codex;
mod dialect;
mod json;
mod packed;
mod render;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Files, Node};
use bilbo_kinds::kind::{Claim, Claims, Kind, Proposal, Refusal};
use bilbo_kinds::reader::{Passage, Reader};
use bilbo_settings::file::File;
use bilbo_settings::key::{About, Default, Key, Source};
use bilbo_settings::size;
use bilbo_store::id::Id;

use crate::dialect::Dialect;
use crate::json::Value;

const NAME: &str = "session";

/// What this kind renders: the passages, the preface and the entry's labels; changes with any change to that.
pub const RENDER: &str = "2";

pub const UNPACK_FLOOR: Key<usize> = Key {
    about: About {
        name: "session.unpack_floor",
        variable: Some("BILBO_SESSION_UNPACK_FLOOR"),
        default: Default::Toml(r#""128 MiB""#),
        allowed: "between 8 MiB and 2 GiB",
        doc: "The count a packed session file may always unpack to, whatever it takes on disk: its bytes, plus what keeping each line holds. A run holds about twice this to keep such a file, so lower it where memory is short.",
    },
    parse: |value| {
        size::read(value)
            .filter(|bytes| (8 * size::MIB..=2 * size::GIB).contains(bytes))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or_else(|| value.refuse())
    },
    show: |bytes| size::show(u64::try_from(*bytes).unwrap_or(u64::MAX)),
};

pub const UNPACK_RATIO: Key<usize> = Key {
    about: About {
        name: "session.unpack_ratio",
        variable: Some("BILBO_SESSION_UNPACK_RATIO"),
        default: Default::Toml("256"),
        allowed: "a whole number from 64 to 4096",
        doc: "How many times its size on disk a packed session file may unpack to, counted the same way, when that is more than session.unpack_floor. A session packs to a few tens of times less.",
    },
    parse: |value| {
        value
            .integer()
            .and_then(|ratio| usize::try_from(ratio).ok())
            .filter(|ratio| (64..=4096).contains(ratio))
            .ok_or_else(|| value.refuse())
    },
    show: ToString::to_string,
};

/// Claude Code's and Codex's transcripts, and Cursor's Claude-shaped ones: one record per line.
#[derive(Debug, Clone, Copy)]
pub struct Session {
    unpack_floor: u64,
    unpack_ratio: u64,
}

impl Session {
    pub fn new(unpack_floor: usize, unpack_ratio: usize) -> Self {
        Self {
            unpack_floor: u64::try_from(unpack_floor).unwrap_or(u64::MAX),
            unpack_ratio: u64::try_from(unpack_ratio).unwrap_or(u64::MAX),
        }
    }
}

impl std::default::Default for Session {
    fn default() -> Self {
        let file = File::default();
        let source = Source {
            var: &|_| None,
            file: &file,
        };
        let floor = UNPACK_FLOOR
            .resolve(&source)
            .expect("the default is a literal its key's unit test parses");
        let ratio = UNPACK_RATIO
            .resolve(&source)
            .expect("the default is a literal its key's unit test parses");
        Self::new(floor.value, ratio.value)
    }
}

impl Reader for Session {
    fn kind(&self) -> &'static str {
        NAME
    }

    fn passages(&self, records: &[&[u8]]) -> Vec<Passage> {
        match dialect::sniff(records.iter().copied()) {
            Some(Dialect::ClaudeCode | Dialect::Cursor) => claude::read::passages(records),
            Some(Dialect::Codex) => codex::read::passages(records),
            None => Vec::new(),
        }
    }

    fn appends_only(&self) -> bool {
        true
    }

    fn entry_labels(&self, records: &[&[u8]]) -> Vec<String> {
        match dialect::sniff(records.iter().copied()) {
            Some(Dialect::ClaudeCode | Dialect::Cursor) => claude::read::labels(records),
            Some(Dialect::Codex) => codex::read::labels(records),
            None => Vec::new(),
        }
    }

    fn preface(&self, records: &[&[u8]]) -> Option<String> {
        let started = records
            .iter()
            .filter_map(|record| json::parse(record).filter(Value::is_dict))
            .find_map(|record| render::time(record.get("timestamp")));
        Some(match started {
            Some(time) => format!("Session started {} UTC:", render::minute(time)),
            None => "Session date unknown:".to_owned(),
        })
    }
}

impl Kind for Session {
    fn name(&self) -> &'static str {
        NAME
    }

    fn summary(&self) -> &'static str {
        "Claude Code and Codex sessions, one record per JSONL line"
    }

    /// Main sessions first, then subagents, so a subagent's session is kept before it.
    fn claims(&self, files: &dyn Files, target: &Path) -> Claims {
        let mut walk = Walk::default();
        match files.node(target) {
            Ok(Node::File) => walk.file(target),
            Ok(Node::Folder) => {
                let below = target.ancestors().any(claude::is_subagents);
                walk.folder(files, target, below);
            }
            Ok(Node::Other) => {}
            Err(error) => walk.gone(files, target, &error),
        }
        walk.sessions.append(&mut walk.subagents);
        Claims {
            entries: walk.sessions,
            refused: walk.refused,
        }
    }

    /// Complete lines only: a line still being written is kept by a later run.
    fn split(&self, bytes: &[u8]) -> Result<Vec<usize>, Refusal> {
        let complete = bytes
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |last| last + 1);
        if let Err(error) = std::str::from_utf8(&bytes[..complete]) {
            return Err(Refusal(format!("not UTF-8 (byte {})", error.valid_up_to())));
        }
        let lines = || bytes[..complete].split_inclusive(|&byte| byte == b'\n');
        if complete > 0 && dialect::sniff(lines()).is_none() {
            return Err(Refusal(
                "no line is a Claude Code, Codex or Cursor record".to_owned(),
            ));
        }
        Ok(lines()
            .scan(0, |end, line| {
                *end += line.len();
                Some(*end)
            })
            .collect())
    }

    fn unpack(&self, file: &Path, bytes: Vec<u8>) -> Result<Vec<u8>, Refusal> {
        if packed::plain(file).is_some() {
            packed::unpack(
                &bytes,
                packed::bound(bytes.len(), self.unpack_floor, self.unpack_ratio),
            )
        } else {
            Ok(bytes)
        }
    }

    fn propose(&self, label: &Path, _bytes: &[u8], kept: &dyn Fn(&Path) -> Option<Id>) -> Proposal {
        Proposal {
            id: None,
            id_label: claude::session_of(label).and_then(|session| kept(&session)),
        }
    }
}

#[derive(Default)]
struct Walk {
    sessions: Vec<Claim>,
    subagents: Vec<Claim>,
    seen: HashSet<PathBuf>,
    refused: Vec<(PathBuf, String)>,
}

impl Walk {
    /// Ends in a UUID and `.jsonl`: `<uuid>.jsonl`, `rollout-<time>-<uuid>.jsonl`,
    /// `<time>--<uuid>.jsonl`, each also packed. Codex publishes a rollout's other form before it removes the first,
    /// so one of the two is there at every instant, and a run that looks at them in turn can still miss both once.
    fn file(&mut self, path: &Path) {
        let label = packed::plain(path).unwrap_or_else(|| path.to_path_buf());
        let subagent = claude::session_of(&label).is_some();
        if !(subagent || is_session_name(&label)) || !self.seen.insert(label.clone()) {
            return;
        }
        let claim = Claim::file(label.clone()).or(packed::beside(&label));
        if subagent {
            self.subagents.push(claim);
        } else {
            self.sessions.push(claim);
        }
    }

    /// A name that was there when listed or asked for and is not now may have changed form since.
    fn gone(&mut self, files: &dyn Files, path: &Path, error: &std::io::Error) {
        if error.kind() == std::io::ErrorKind::NotFound {
            let label = packed::plain(path).unwrap_or_else(|| path.to_path_buf());
            let other = packed::sibling(path);
            if (claude::session_of(&label).is_some() || is_session_name(&label))
                && matches!(files.node(&other), Ok(Node::File))
            {
                return self.file(&other);
            }
        }
        self.refuse(path, error);
    }

    /// `below_subagents`: nothing is skipped under `subagents/`.
    fn folder(&mut self, files: &dyn Files, folder: &Path, below_subagents: bool) {
        let names = match files.list(folder) {
            Ok(names) => names,
            Err(error) => return self.refuse(folder, &error),
        };
        for name in names {
            let bytes = name.as_encoded_bytes();
            if bytes.first() == Some(&b'.')
                || (!below_subagents && claude::SKIPPED.iter().any(|skip| skip.as_bytes() == bytes))
            {
                continue;
            }
            let path = folder.join(&name);
            match files.node(&path) {
                Ok(Node::File) => self.file(&path),
                Ok(Node::Folder) => {
                    let below = below_subagents || claude::is_subagents(&path);
                    self.folder(files, &path, below);
                }
                Ok(Node::Other) => {}
                Err(error) => self.gone(files, &path, &error),
            }
        }
    }

    fn refuse(&mut self, path: &Path, error: &std::io::Error) {
        self.refused
            .push((path.to_path_buf(), format!("cannot read: {error}")));
    }
}

fn is_session_name(label: &Path) -> bool {
    label
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".jsonl"))
        .is_some_and(|stem| {
            let stem = stem.as_bytes();
            stem.len() >= 36 && is_uuid(&stem[stem.len() - 36..])
        })
}

fn is_uuid(bytes: &[u8]) -> bool {
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(at, &byte)| match at {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use bilbo_kinds::files::{Call, Files, Memory, Node, Turning};
    use bilbo_kinds::kind::{Kind, Proposal, Refusal};
    use bilbo_kinds::reader::{Passage, Reader};
    use bilbo_store::id::Id;

    use bilbo_settings::file::File;
    use bilbo_settings::key::Source;

    use super::{Session, UNPACK_FLOOR, UNPACK_RATIO, is_uuid, packed};

    const CLAUDE: &[u8] = include_bytes!("../fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl");
    const SUBAGENT: &[u8] = include_bytes!("../fixtures/agent-ad63c2b3c9e7a4b3d.jsonl");
    const META: &[u8] = include_bytes!("../fixtures/agent-ad63c2b3c9e7a4b3d.meta.json");
    const CODEX: &[u8] = include_bytes!(
        "../fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
    );
    const CODEX_PACKED: &[u8] = include_bytes!(
        "../fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl.zst"
    );
    const CURSOR: &[u8] = include_bytes!("../fixtures/cursor-session.jsonl");
    const NONE: &[u8] = include_bytes!("../fixtures/not-a-session.jsonl");
    const NOT_UTF8: &[u8] = include_bytes!("../fixtures/not-utf8.jsonl");

    const MAIN: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
    const OTHER: &str = "0e1d2c3b-4a59-4687-b6a5-a4b3c2d1e0f9";

    fn claimed(files: &Memory, target: &str) -> Vec<String> {
        Session::default()
            .claims(files, Path::new(target))
            .entries
            .iter()
            .map(|claim| claim.label().to_str().unwrap_or_default().to_owned())
            .collect()
    }

    fn projects() -> Memory {
        let p = "/h/.claude/projects";
        Memory::new()
            .file(format!("{p}/-h-src-garden/{MAIN}.jsonl"), CLAUDE)
            .file(
                format!("{p}/-h-src-garden/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl"),
                SUBAGENT,
            )
            .file(
                format!("{p}/-h-src-garden/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.meta.json"),
                META,
            )
            .file(
                format!("{p}/-h-src-garden/{MAIN}/subagents/workflows/wf_1/agent-b2.jsonl"),
                SUBAGENT,
            )
            .file(
                format!("{p}/-h-src-garden/{MAIN}/subagents/workflows/wf_1/agent-b2.meta.json"),
                META,
            )
            .file(
                format!("{p}/-h-src-garden/{MAIN}/subagents/workflows/wf_1/journal.jsonl"),
                NONE,
            )
            .file(
                format!("{p}/-h-src-garden/{MAIN}/tool-results/toolu_1.txt"),
                "out",
            )
            .file(
                format!("{p}/-h-src-garden/{MAIN}/tool-results/{OTHER}.jsonl"),
                CLAUDE,
            )
            .file(format!("{p}/-h-src-garden/{MAIN}/workflows/run.json"), "{}")
            .file(
                format!("{p}/-h-src-garden/{MAIN}/workflows/{OTHER}.jsonl"),
                CLAUDE,
            )
            .file(format!("{p}/-h-src-garden/memory/MEMORY.md"), "- a memory")
            .file(format!("{p}/-h-src-garden/memory/{OTHER}.jsonl"), CLAUDE)
            .file(format!("{p}/-h-src-garden/.hidden/{OTHER}.jsonl"), CLAUDE)
            .file(format!("{p}/-h-src-garden/.{OTHER}.jsonl"), CLAUDE)
            .file(format!("{p}/-h-src-garden/notes.jsonl"), NONE)
            .other(format!("{p}/-h-src-garden/{OTHER}.jsonl"))
            .file(format!("{p}/-h-src-tools/{OTHER}.jsonl"), CLAUDE)
    }

    #[test]
    fn a_projects_folder_gives_its_sessions_then_their_subagents_workflow_runs_included() {
        let p = "/h/.claude/projects";
        assert_eq!(
            claimed(&projects(), p),
            [
                format!("{p}/-h-src-garden/{MAIN}.jsonl"),
                format!("{p}/-h-src-tools/{OTHER}.jsonl"),
                format!("{p}/-h-src-garden/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl"),
                format!("{p}/-h-src-garden/{MAIN}/subagents/workflows/wf_1/agent-b2.jsonl"),
            ]
        );
        let subagents = format!("{p}/-h-src-garden/{MAIN}/subagents/workflows");
        assert_eq!(
            claimed(&projects(), &subagents),
            [format!("{subagents}/wf_1/agent-b2.jsonl")],
            "a target below subagents/ skips nothing"
        );
        assert!(
            Session::default()
                .claims(&projects(), Path::new(p))
                .refused
                .is_empty()
        );
    }

    #[test]
    fn a_sessions_folder_gives_every_rollout_under_its_date_folders() {
        let s = "/h/.codex/sessions";
        let files = Memory::new()
            .file(
                format!("{s}/2026/11/06/rollout-2026-11-06T09-10-00-{OTHER}.jsonl"),
                CODEX,
            )
            .file(
                format!("{s}/2026/10/31/rollout-2026-10-31T23-59-59-{MAIN}.jsonl"),
                CODEX,
            )
            .file(format!("{s}/2026/10/31/notes.txt"), "x")
            .folder(format!("{s}/2026/12"));
        assert_eq!(
            claimed(&files, s),
            [
                format!("{s}/2026/10/31/rollout-2026-10-31T23-59-59-{MAIN}.jsonl"),
                format!("{s}/2026/11/06/rollout-2026-11-06T09-10-00-{OTHER}.jsonl"),
            ]
        );
    }

    #[test]
    fn a_named_file_is_claimed_only_when_its_name_is_a_sessions() {
        let files = projects().file(format!("/c/2026-02-14T233343Z--{OTHER}.jsonl"), CLAUDE);
        let p = "/h/.claude/projects/-h-src-garden";
        for (target, taken) in [
            (format!("{p}/{MAIN}.jsonl"), true),
            (
                format!("{p}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl"),
                true,
            ),
            (format!("/c/2026-02-14T233343Z--{OTHER}.jsonl"), true),
            (
                format!("{p}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.meta.json"),
                false,
            ),
            (format!("{p}/notes.jsonl"), false),
            (format!("{p}/memory/MEMORY.md"), false),
        ] {
            assert_eq!(
                claimed(&files, &target).len(),
                usize::from(taken),
                "{target}"
            );
        }
    }

    #[test]
    fn an_unreadable_folder_is_refused_and_the_walk_goes_on() {
        let files = projects().unreadable("/h/.claude/projects/-h-locked", Node::Folder);
        let claims = Session::default().claims(&files, Path::new("/h/.claude/projects"));
        assert_eq!(claims.entries.len(), 4);
        assert_eq!(claims.refused.len(), 1);
        assert_eq!(
            claims.refused[0].0,
            Path::new("/h/.claude/projects/-h-locked")
        );
        assert!(claims.refused[0].1.starts_with("cannot read: "));
        assert_eq!(
            Session::default()
                .claims(&Memory::new(), Path::new("/nope"))
                .refused
                .len(),
            1
        );
    }

    #[test]
    fn each_line_is_a_record_and_the_records_join_back_to_the_file() {
        for bytes in [CLAUDE, SUBAGENT, CODEX, CURSOR] {
            let ends = Session::default().split(bytes).expect("a session");
            assert_eq!(ends.last().copied(), Some(bytes.len()));
            assert_eq!(ends.len(), bytes.iter().filter(|&&b| b == b'\n').count());
        }
        assert_eq!(Session::default().split(b""), Ok(vec![]));
    }

    /// A departure from the eval's render, which read a lone carriage return as a line end.
    #[test]
    fn a_lone_carriage_return_does_not_end_a_record() {
        use bilbo_kinds::reader::Reader;
        let entry =
            |text: &str| format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{text}\"}}}}");
        let bytes = format!("{}\r{}\n{}\r\n", entry("a"), entry("b"), entry("c")).into_bytes();
        let ends = Session::default().split(&bytes).expect("a session");
        assert_eq!(ends.len(), 2);
        let records: Vec<&[u8]> = bytes.split_inclusive(|&byte| byte == b'\n').collect();
        let read: Vec<(u64, String)> = Session::default()
            .passages(&records)
            .into_iter()
            .map(|passage| (passage.range.first, passage.text))
            .collect();
        assert_eq!(read, [(2, "user: c".to_owned())]);
    }

    #[test]
    fn a_line_still_being_written_is_left_for_a_later_run() {
        let grown = [CURSOR, b"{\"role\":\"user\",\"mess"].concat();
        assert_eq!(
            Session::default().split(&grown),
            Session::default().split(CURSOR)
        );
        assert_eq!(Session::default().split(b"{\"role\":\"us"), Ok(vec![]));
        let cut_char = [
            CURSOR,
            "{\"role\":\"é".as_bytes().split_last().expect("bytes").1,
        ]
        .concat();
        assert_eq!(
            Session::default().split(&cut_char),
            Session::default().split(CURSOR)
        );
    }

    #[test]
    fn a_file_that_is_not_utf8_or_not_a_session_is_refused() {
        assert_eq!(
            Session::default().split(NOT_UTF8),
            Err(Refusal(format!(
                "not UTF-8 (byte {})",
                NOT_UTF8
                    .iter()
                    .position(|&b| b == 0xff)
                    .expect("a bad byte")
            )))
        );
        assert_eq!(
            Session::default().split(NONE),
            Err(Refusal(
                "no line is a Claude Code, Codex or Cursor record".to_owned()
            ))
        );
        let one_bad_line = [b"not json\n", CODEX].concat();
        assert_eq!(
            Session::default()
                .split(&one_bad_line)
                .map(|ends| ends.len()),
            Ok(1 + CODEX.iter().filter(|&&b| b == b'\n').count())
        );
    }

    #[test]
    fn a_subagent_proposes_its_sessions_entry_as_its_id_label() {
        let held = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
        let asked = RefCell::new(Vec::new());
        let kept = |path: &Path| {
            asked.borrow_mut().push(path.to_path_buf());
            Some(held)
        };
        let p = "/h/.claude/projects/-h-src-garden";
        let subagent = format!("{p}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl");
        assert_eq!(
            Session::default().propose(Path::new(&subagent), SUBAGENT, &kept),
            Proposal {
                id: None,
                id_label: Some(held)
            }
        );
        assert_eq!(
            *asked.borrow(),
            [PathBuf::from(format!("{p}/{MAIN}.jsonl"))]
        );
        let main = format!("{p}/{MAIN}.jsonl");
        assert_eq!(
            Session::default().propose(Path::new(&main), CLAUDE, &kept),
            Proposal::default()
        );
        assert_eq!(
            Session::default().propose(Path::new(&subagent), SUBAGENT, &|_| None),
            Proposal::default()
        );
    }

    #[test]
    fn a_uuid_is_8_4_4_4_12_hex_digits() {
        assert!(is_uuid(MAIN.as_bytes()));
        assert!(is_uuid(MAIN.to_uppercase().as_bytes()));
        assert!(!is_uuid(&MAIN.as_bytes()[1..]));
        assert!(!is_uuid(MAIN.replace('-', "_").as_bytes()));
        assert!(!is_uuid(MAIN.replace('c', "g").as_bytes()));
    }

    #[test]
    fn a_session_is_read_by_the_dialect_its_records_show() {
        use bilbo_kinds::reader::Reader;
        let first = |bytes: &[u8]| {
            let records: Vec<&[u8]> = bytes.split_inclusive(|&byte| byte == b'\n').collect();
            Session::default()
                .passages(&records)
                .first()
                .map(|passage| (passage.range.first, passage.text.clone()))
        };
        assert_eq!(Reader::kind(&Session::default()), Session::default().name());
        assert_eq!(
            first(CLAUDE),
            Some((
                2,
                "user: Plant the tomatoes in the north bed and log the commit.".to_owned()
            ))
        );
        assert_eq!(
            first(SUBAGENT),
            Some((1, "user: Count the seed packets in the shed.".to_owned()))
        );
        assert_eq!(
            first(CURSOR),
            Some((1, "user: Rename the trellis module.".to_owned()))
        );
        assert_eq!(
            first(CODEX),
            Some((
                3,
                "user: Water the beans, then show me the bed.\nuser: [input_image]".to_owned()
            ))
        );
        assert_eq!(first(NONE), None);
        assert_eq!(first(b""), None);
        assert_eq!(first(NOT_UTF8), None);
    }

    #[test]
    fn a_session_whose_only_evidence_is_odd_still_reaches_its_reader() {
        use bilbo_kinds::reader::Reader;
        let texts = |bytes: &[u8]| -> Vec<String> {
            let records: Vec<&[u8]> = bytes.split_inclusive(|&byte| byte == b'\n').collect();
            assert!(
                Session::default().split(bytes).is_ok(),
                "the file is claimed"
            );
            Session::default()
                .passages(&records)
                .into_iter()
                .map(|passage| passage.text)
                .collect()
        };
        let null_type = b"{\"type\":null,\"role\":\"user\",\"message\":{\"content\":\"hello\"}}\n";
        assert_eq!(texts(null_type), ["user: hello"]);
        let nan = b"{\"type\":\"user\",\"n\":NaN,\"message\":{\"content\":\"hello\"}}\n";
        assert_eq!(texts(nan), ["user: hello"]);
        let prelude = [b"{\"type\":\"garbage\"}\n".as_slice(), CODEX].concat();
        assert_eq!(
            texts(&prelude)[0],
            "user: Water the beans, then show me the bed.\nuser: [input_image]",
            "an unknown line first does not hide a Codex session"
        );
        for weak in [
            b"{\"type\":null,\"role\":\"garbage\"}\n".as_slice(),
            b"{\"type\":\"user\",\"n\":NaN}\n",
        ] {
            assert_eq!(
                texts(&[weak, CODEX].concat())[0],
                "user: Water the beans, then show me the bed.\nuser: [input_image]",
                "{}",
                String::from_utf8_lossy(weak)
            );
        }
    }

    struct Dice(u64);

    impl Dice {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let drawn = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33;
            usize::try_from(drawn).unwrap_or(0) % bound.max(1)
        }

        fn byte(&mut self) -> u8 {
            const OFTEN: &[u8] = b"{}[]\",:\\ \n\r#-=>`~|0159eEuNI\xff\xc3\x80";
            if self.below(3) == 0 {
                u8::try_from(self.below(256)).unwrap_or(0)
            } else {
                OFTEN[self.below(OFTEN.len())]
            }
        }

        fn bytes(&mut self, most: usize) -> Vec<u8> {
            (0..self.below(most)).map(|_| self.byte()).collect()
        }

        fn spoiled(&mut self, line: &[u8]) -> Vec<u8> {
            let mut line = line.to_vec();
            for _ in 0..=self.below(3) {
                let at = self.below(line.len());
                match self.below(7) {
                    0 if !line.is_empty() => line[at] = self.byte(),
                    1 if !line.is_empty() => {
                        line.remove(at);
                    }
                    2 => line.insert(at, self.byte()),
                    3 => line.truncate(at),
                    4 => {
                        let run = vec![self.byte(); self.below(300)];
                        line.splice(at..at, run);
                    }
                    5 => {
                        const PARTS: [&[u8]; 10] = [
                            b"\\u",
                            b"\\uab\xe2\x82\xac",
                            b"\\ud800",
                            b"NaN",
                            b"1e400",
                            b"[[[[[[[[",
                            b"\"\\",
                            b"\n---\n",
                            b"```",
                            b"> - ",
                        ];
                        let part = PARTS[self.below(PARTS.len())];
                        line.splice(at..at, part.iter().copied());
                    }
                    _ => {
                        let end = (at + self.below(40)).min(line.len());
                        let again = line[at..end].to_vec();
                        line.splice(at..at, again);
                    }
                }
            }
            line
        }
    }

    fn sound(passages: &[Passage], records: usize, case: usize) {
        let mut next = 1;
        for passage in passages {
            let range = passage.range;
            assert!(
                range.first >= next && range.count > 0,
                "case {case}: {range:?}"
            );
            next = range.first + range.count;
            assert!(
                next - 1 <= records as u64,
                "case {case}: {range:?} of {records}"
            );
            assert_eq!(passage.ends.len() as u64, range.count, "case {case}");
            assert_eq!(
                passage.ends.last(),
                Some(&passage.text.len()),
                "case {case}"
            );
            assert!(passage.ends.is_sorted(), "case {case}");
            let starts = passage.pieces.iter().map(|piece| piece.start);
            assert!(
                passage
                    .ends
                    .iter()
                    .copied()
                    .chain(starts)
                    .all(|at| passage.text.is_char_boundary(at)),
                "case {case}"
            );
            assert!(
                passage.pieces.iter().map(|piece| piece.start).is_sorted(),
                "case {case}"
            );
        }
    }

    #[test]
    fn no_record_makes_a_reader_panic_or_leave_its_generation() {
        use bilbo_kinds::reader::Reader;
        let lines: Vec<&[u8]> = [CLAUDE, SUBAGENT, CODEX, CURSOR, NONE, NOT_UTF8]
            .into_iter()
            .flat_map(|file| file.split_inclusive(|&byte| byte == b'\n'))
            .collect();
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15);
        let mut rendered = 0;
        for case in 0..4000 {
            let records: Vec<Vec<u8>> = (0..dice.below(7))
                .map(|_| match dice.below(5) {
                    0 => dice.bytes(80),
                    1 => lines[dice.below(lines.len())].to_vec(),
                    _ => {
                        let line = lines[dice.below(lines.len())];
                        dice.spoiled(line)
                    }
                })
                .collect();
            let records: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
            for passages in [
                Session::default().passages(&records),
                crate::claude::read::passages(&records),
                crate::codex::read::passages(&records),
            ] {
                sound(&passages, records.len(), case);
                assert!(passages.iter().all(|passage| passage.range.count == 1));
                rendered += passages.len();
            }
        }
        assert!(
            rendered > 2000,
            "the cases still reach the renders: {rendered}"
        );
    }

    fn preface(text: &str) -> Option<String> {
        let records: Vec<&[u8]> = text.split_inclusive('\n').map(str::as_bytes).collect();
        Session::default().preface(&records)
    }

    fn stamped(timestamp: &str) -> String {
        format!("{{\"timestamp\":\"{timestamp}\",\"type\":\"summary\"}}\n")
    }

    #[test]
    fn the_preface_is_the_first_stamped_record_also_one_that_renders_nothing() {
        let text = format!(
            "{}{}{}",
            stamped("2026-11-07T16:39:59Z"),
            r#"{"type":"user","timestamp":"2026-11-07T16:45:00Z","message":{"role":"user","content":"hello"}}"#.to_owned() + "\n",
            stamped("2026-11-08T01:00:00Z"),
        );
        let records: Vec<&[u8]> = text.split_inclusive('\n').map(str::as_bytes).collect();
        let first = Session::default().passages(&records);
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0].range.first, 2,
            "the first passage is the second record"
        );
        assert_eq!(
            Session::default().preface(&records).as_deref(),
            Some("Session started 2026-11-07 16:39 UTC:")
        );
    }

    #[test]
    fn a_session_with_no_timestamp_says_its_date_is_unknown() {
        let unknown = Some("Session date unknown:".to_owned());
        for text in [
            "",
            "{\"type\":\"user\"}\n",
            "{\"timestamp\":\"yesterday\"}\n",
            "{\"timestamp\":\"\"}\n{\"timestamp\":null}\n{\"timestamp\":5}\n",
            "[1,2]\nnot json\n\"2026-11-07T16:39:12Z\"\n",
            "{\"timestamp\":\"1969-12-31T23:59:59Z\"}\n",
        ] {
            assert_eq!(preface(text), unknown, "{text:?}");
        }
        assert_eq!(
            preface("{\"timestamp\":\"nope\"}\n{\"timestamp\":\"2026-11-07T16:39:12Z\"}\n")
                .as_deref(),
            Some("Session started 2026-11-07 16:39 UTC:"),
            "an unreadable stamp is passed over"
        );
    }

    #[test]
    fn the_preface_is_utc_to_the_minute() {
        for (stamp, line) in [
            ("2026-11-07T16:39:12Z", "2026-11-07 16:39"),
            ("2026-11-07T16:39:59.999Z", "2026-11-07 16:39"),
            ("2026-11-07T16:39:00.000Z", "2026-11-07 16:39"),
            ("2026-11-07T16:39:12+02:00", "2026-11-07 14:39"),
            ("2026-11-07T16:39:12-07:00", "2026-11-07 23:39"),
            ("2026-11-07T00:10:00+02:00", "2026-11-06 22:10"),
            ("2026-11-07T16:39:12", "2026-11-07 16:39"),
            ("2026-11-07 16:39:12.5", "2026-11-07 16:39"),
            ("2024-02-29T23:59:59Z", "2024-02-29 23:59"),
            ("2024-02-28T23:30:00-01:00", "2024-02-29 00:30"),
            ("2025-12-31T23:59:59Z", "2025-12-31 23:59"),
            ("2025-12-31T23:59:59-00:01", "2026-01-01 00:00"),
            ("1970-01-01T00:00:00Z", "1970-01-01 00:00"),
            ("2100-03-01T00:00:00Z", "2100-03-01 00:00"),
        ] {
            assert_eq!(
                preface(&stamped(stamp)),
                Some(format!("Session started {line} UTC:")),
                "{stamp}"
            );
        }
    }

    #[test]
    fn both_dialects_and_cursor_give_the_same_line_for_the_same_first_timestamp() {
        let same = |fixture: &[u8]| {
            let records: Vec<&[u8]> = fixture.split_inclusive(|&byte| byte == b'\n').collect();
            Session::default().preface(&records)
        };
        assert_eq!(
            same(CLAUDE).as_deref(),
            Some("Session started 2026-11-02 10:06 UTC:")
        );
        assert_eq!(
            same(SUBAGENT).as_deref(),
            Some("Session started 2026-11-02 10:01 UTC:")
        );
        assert_eq!(
            same(CODEX).as_deref(),
            Some("Session started 2026-11-06 09:10 UTC:")
        );
        assert_eq!(same(CURSOR).as_deref(), Some("Session date unknown:"));
        let moved = |fixture: &[u8], from: &str, to: &str| {
            let text = String::from_utf8(fixture.to_vec())
                .expect("utf-8")
                .replacen(from, to, 1);
            same(text.as_bytes())
        };
        assert_eq!(
            moved(
                CLAUDE,
                "2026-11-02T10:06:00.000Z",
                "2026-11-09T04:05:06.000Z"
            ),
            moved(
                CODEX,
                "2026-11-06T09:10:00.000Z",
                "2026-11-09T04:05:06.000Z"
            ),
        );
    }

    #[test]
    fn the_kind_is_named_session_with_its_summary() {
        assert_eq!(Session::default().name(), "session");
        assert_eq!(
            Session::default().summary(),
            "Claude Code and Codex sessions, one record per JSONL line"
        );
    }

    const SESSIONS: &str = "/h/.codex/sessions/2026/11/06";

    fn rollout() -> String {
        format!("{SESSIONS}/rollout-2026-11-06T09-10-00-{OTHER}.jsonl")
    }

    fn entries(files: &dyn Files, target: &str) -> Vec<(PathBuf, Vec<PathBuf>)> {
        Session::default()
            .claims(files, Path::new(target))
            .entries
            .into_iter()
            .map(|claim| (claim.label().to_path_buf(), claim.files().to_vec()))
            .collect()
    }

    #[test]
    fn a_packed_rollout_is_claimed_under_its_plain_path() {
        let files = Memory::new().file(format!("{}.zst", rollout()), CODEX_PACKED);
        let claims = Session::default().claims(&files, Path::new(SESSIONS));
        assert_eq!(claims.entries.len(), 1);
        assert_eq!(claims.entries[0].label(), Path::new(&rollout()));
        assert!(claims.refused.is_empty());
    }

    #[test]
    fn a_rollout_has_one_label_whether_plain_or_packed() {
        let plain = Memory::new().file(rollout(), CODEX);
        let packed = Memory::new().file(format!("{}.zst", rollout()), CODEX_PACKED);
        assert_eq!(entries(&plain, SESSIONS), entries(&packed, SESSIONS));
    }

    #[test]
    fn a_rollout_in_both_forms_is_one_claim_plain_first() {
        let files = Memory::new()
            .file(rollout(), CODEX)
            .file(format!("{}.zst", rollout()), CODEX_PACKED);
        assert_eq!(
            entries(&files, SESSIONS),
            [(
                PathBuf::from(rollout()),
                vec![
                    PathBuf::from(rollout()),
                    PathBuf::from(format!("{}.zst", rollout()))
                ]
            )]
        );
    }

    #[test]
    fn every_claim_names_its_plain_file_then_its_packed_one() {
        let p = "/h/.claude/projects/-h-src-garden";
        let subagent = format!("{p}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl");
        let files = projects().file(rollout(), CODEX);
        for target in [format!("{p}/{MAIN}.jsonl"), subagent, rollout()] {
            let claims = entries(&files, &target);
            assert_eq!(
                claims,
                [(
                    PathBuf::from(&target),
                    vec![
                        PathBuf::from(&target),
                        PathBuf::from(format!("{target}.zst"))
                    ]
                )],
                "{target}"
            );
        }
    }

    #[test]
    fn a_packed_session_of_any_name_form_and_a_packed_subagent_are_claimed() {
        let p = "/h/.claude/projects/-h-src-garden";
        let files = Memory::new()
            .file(format!("{p}/{MAIN}.jsonl.zst"), "z")
            .file(format!("/c/2026-02-14T233343Z--{OTHER}.jsonl.zst"), "z")
            .file(
                format!("{p}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl.zst"),
                "z",
            )
            .file(format!("{p}/notes.jsonl.zst"), "z")
            .file(format!("{p}/x.zst"), "z");
        for (target, taken) in [
            (format!("{p}/{MAIN}.jsonl.zst"), true),
            (format!("/c/2026-02-14T233343Z--{OTHER}.jsonl.zst"), true),
            (
                format!("{p}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl.zst"),
                true,
            ),
            (format!("{p}/notes.jsonl.zst"), false),
            (format!("{p}/x.zst"), false),
        ] {
            assert_eq!(
                claimed(&files, &target).len(),
                usize::from(taken),
                "{target}"
            );
        }
        assert_eq!(
            claimed(&files, p).len(),
            2,
            "the folder walk finds the same"
        );
    }

    #[test]
    fn a_named_packed_file_is_claimed_as_its_rollout() {
        let files = Memory::new().file(format!("{}.zst", rollout()), CODEX_PACKED);
        assert_eq!(claimed(&files, &format!("{}.zst", rollout())), [rollout()]);
    }

    #[test]
    fn a_rollout_that_changes_form_between_the_listing_and_the_look_is_claimed() {
        let plain = || Memory::new().file(rollout(), CODEX);
        let packed = || Memory::new().file(format!("{}.zst", rollout()), CODEX_PACKED);
        for (before, after) in [(plain(), packed()), (packed(), plain())] {
            let files = Turning::new(before, after, Call::List, SESSIONS);
            let claims = Session::default().claims(&files, Path::new(SESSIONS));
            assert_eq!(claims.entries.len(), 1);
            assert_eq!(claims.entries[0].label(), Path::new(&rollout()));
            assert!(claims.refused.is_empty());
        }
    }

    #[test]
    fn a_name_whose_node_cannot_be_read_is_refused_and_its_other_form_is_not_claimed() {
        let files = Memory::new()
            .denied(rollout())
            .file(format!("{}.zst", rollout()), CODEX_PACKED);
        let claims = Session::default().claims(&files, Path::new(&rollout()));
        assert!(claims.entries.is_empty());
        assert_eq!(claims.refused.len(), 1);
        assert_eq!(claims.refused[0].0, Path::new(&rollout()));
        assert_eq!(claims.refused[0].1, "cannot read: permission denied");
    }

    #[test]
    fn a_named_rollout_that_is_gone_is_claimed_through_its_other_form() {
        let packed = Memory::new().file(format!("{}.zst", rollout()), CODEX_PACKED);
        let plain = Memory::new().file(rollout(), CODEX);
        for (files, target) in [(&packed, rollout()), (&plain, format!("{}.zst", rollout()))] {
            let claims = Session::default().claims(files, Path::new(&target));
            assert_eq!(claims.entries.len(), 1, "{target}");
            assert_eq!(claims.entries[0].label(), Path::new(&rollout()));
            assert!(claims.refused.is_empty());
        }
    }

    #[test]
    fn a_name_that_is_gone_with_no_other_form_is_refused_as_before() {
        let p = "/h/.claude/projects/-h-src-garden";
        let files = Memory::new().other(format!("{}.zst", rollout())).folder(p);
        for target in [
            rollout(),
            format!("{p}/{MAIN}.jsonl"),
            format!("{p}/notes.jsonl"),
        ] {
            let claims = Session::default().claims(&files, Path::new(&target));
            assert!(claims.entries.is_empty(), "{target}");
            assert_eq!(claims.refused.len(), 1, "{target}");
            assert!(claims.refused[0].1.starts_with("cannot read: "));
        }
    }

    #[test]
    fn codexs_temporary_files_are_not_claimed() {
        let files = Memory::new()
            .file(format!("{SESSIONS}/rollout-compress-ab12.tmp"), "z")
            .file(format!("{SESSIONS}/.rollout-materialize-ab12"), "z");
        let claims = Session::default().claims(&files, Path::new(SESSIONS));
        assert!(claims.entries.is_empty());
        assert!(claims.refused.is_empty());
    }

    #[test]
    fn the_packed_fixtures_unpack_to_the_plain_ones_byte_for_byte() {
        const OTHER_PLAIN: &[u8] = include_bytes!(
            "../fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
        );
        const OTHER_PACKED: &[u8] = include_bytes!(
            "../fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl.zst"
        );
        for (plain, packed) in [(CODEX, CODEX_PACKED), (OTHER_PLAIN, OTHER_PACKED)] {
            assert_eq!(
                packed::unpack(packed, packed::bound(packed.len(), 128 << 20, 256))
                    .expect("unpacks"),
                plain
            );
        }
    }

    #[test]
    fn a_packed_file_is_unpacked_and_a_plain_one_handed_on_as_read() {
        assert_eq!(
            Session::default().unpack(
                Path::new(&format!("{}.zst", rollout())),
                CODEX_PACKED.to_vec()
            ),
            Ok(CODEX.to_vec())
        );
        assert_eq!(
            Session::default().unpack(Path::new(&rollout()), CODEX_PACKED.to_vec()),
            Ok(CODEX_PACKED.to_vec())
        );
    }

    #[test]
    fn a_file_that_is_not_zstd_or_is_cut_short_is_refused() {
        let file = PathBuf::from(format!("{}.zst", rollout()));
        for bytes in [
            b"not zstd at all".to_vec(),
            CODEX_PACKED[..CODEX_PACKED.len() / 2].to_vec(),
        ] {
            let refusal = Session::default()
                .unpack(&file, bytes)
                .expect_err("refused");
            assert!(refusal.0.starts_with("cannot unpack: "), "{}", refusal.0);
        }
    }

    fn lines(bytes: &[u8]) -> u64 {
        u64::try_from(bytes.iter().filter(|&&byte| byte == b'\n').count()).expect("fits")
    }

    #[test]
    fn a_file_of_exactly_the_bound_passes_and_one_byte_more_is_refused() {
        let limit = u64::try_from(CODEX.len()).expect("fits") + 64 * lines(CODEX);
        assert_eq!(packed::unpack(CODEX_PACKED, limit).expect("passes"), CODEX);
        assert_eq!(
            packed::unpack(CODEX_PACKED, limit - 1),
            Err(Refusal(format!(
                "would hold more than {} bytes unpacked",
                limit - 1
            )))
        );
    }

    #[test]
    fn unpacking_stops_at_the_bound_and_never_reaches_what_follows() {
        let mut bytes = CODEX_PACKED.to_vec();
        bytes.extend_from_slice(b"not zstd at all");
        assert_eq!(
            packed::unpack(&bytes, 16),
            Err(Refusal("would hold more than 16 bytes unpacked".to_owned()))
        );
    }

    #[test]
    fn the_bound_counts_what_keeping_holds_for_every_line() {
        let many = zstd::bulk::compress(&b"x\n".repeat(100), 3).expect("packs");
        assert!(packed::unpack(&many, 6600).is_ok());
        assert!(packed::unpack(&many, 6599).is_err());
        let one = zstd::bulk::compress(&[&[b'x'; 199][..], b"\n"].concat(), 3).expect("packs");
        assert!(packed::unpack(&one, 264).is_ok());
        assert!(packed::unpack(&one, 263).is_err());
    }

    #[test]
    fn the_bound_is_the_floor_or_the_ratio_times_the_packed_size_whichever_is_more() {
        assert_eq!(packed::bound(0, 128 << 20, 256), 128 << 20);
        assert_eq!(packed::bound(1 << 19, 128 << 20, 256), 128 << 20);
        assert_eq!(
            packed::bound((1 << 19) + 1, 128 << 20, 256),
            (128 << 20) + 256
        );
        assert_eq!(packed::bound(usize::MAX, 128 << 20, 256), u64::MAX);
        assert_eq!(packed::bound(0, 8 << 20, 64), 8 << 20);
        assert_eq!(packed::bound(1 << 17, 8 << 20, 64), 8 << 20);
        assert_eq!(packed::bound((1 << 17) + 1, 8 << 20, 64), (8 << 20) + 64);
    }

    #[test]
    fn a_small_file_made_to_hold_far_more_than_its_size_is_refused() {
        let first = CODEX
            .split_inclusive(|&byte| byte == b'\n')
            .next()
            .expect("a line");
        let bomb = |ends: usize| {
            let mut bytes = first.to_vec();
            bytes.resize(first.len() + ends, b'\n');
            bytes
        };
        let name = PathBuf::from(format!("{}.zst", rollout()));
        let refusal = Refusal("would hold more than 134217728 bytes unpacked".to_owned());
        for ends in [14 << 20, 2 << 20] {
            let packed = zstd::bulk::compress(&bomb(ends), 3).expect("packs");
            assert!(packed.len() < 64 << 10);
            assert_eq!(
                Session::default().unpack(&name, packed).err(),
                Some(refusal.clone())
            );
        }
        let packed = zstd::bulk::compress(&bomb(1 << 20), 3).expect("packs");
        let unpacked = Session::default().unpack(&name, packed);
        assert!(
            unpacked == Ok(bomb(1 << 20)),
            "{:?}",
            unpacked.map(|bytes| bytes.len())
        );
    }

    #[test]
    fn a_frame_that_asks_for_a_window_over_128_mib_is_refused() {
        let refusal = packed::unpack(&frame(28), 128 << 20).expect_err("refused");
        assert!(refusal.0.starts_with("cannot unpack: "), "{}", refusal.0);
        assert_eq!(
            packed::unpack(&frame(27), 128 << 20).expect("unpacks"),
            b"some bytes\nsome more bytes\n"
        );
    }

    fn frame(window_log: u32) -> Vec<u8> {
        let mut packed = Vec::new();
        let mut encoder = zstd::stream::write::Encoder::new(&mut packed, 3).expect("encoder");
        encoder.window_log(window_log).expect("window");
        encoder.write_all(b"some bytes\n").expect("first");
        encoder.write_all(b"some more bytes\n").expect("second");
        encoder.finish().expect("finish");
        packed
    }

    #[test]
    fn a_frame_may_ask_for_a_window_up_to_the_bound_and_no_less_than_8_mib() {
        for (limit, fits, too_big) in [
            (8 << 20, 23, 24),
            (12 << 20, 24, 25),
            (1000, 23, 24),
            (2 << 30, 27, 28),
        ] {
            assert!(packed::unpack(&frame(fits), limit).is_ok(), "{limit}");
            let refusal = packed::unpack(&frame(too_big), limit).expect_err("refused");
            assert!(refusal.0.starts_with("cannot unpack: "), "{}", refusal.0);
        }
    }

    fn setting<T>(
        key: &bilbo_settings::key::Key<T>,
        env: Option<&str>,
        text: &str,
    ) -> Result<T, String> {
        let file = File::parse(Path::new("/c/config.toml"), text.as_bytes()).expect("toml");
        let var = |name: &str| {
            (Some(name) == key.about.variable)
                .then(|| env.map(Into::into))
                .flatten()
        };
        key.resolve(&Source {
            var: &var,
            file: &file,
        })
        .map(|resolved| resolved.value)
        .map_err(|error| error.to_string())
    }

    #[test]
    fn the_unpack_floor_is_8_mib_to_2_gib_and_defaults_to_128_mib() {
        assert_eq!(setting(&UNPACK_FLOOR, None, ""), Ok(134_217_728));
        assert_eq!(
            setting(&UNPACK_FLOOR, None, "session.unpack_floor = \"16 MiB\""),
            Ok(16 << 20)
        );
        assert_eq!(
            setting(
                &UNPACK_FLOOR,
                Some("9 MiB"),
                "session.unpack_floor = \"16 MiB\""
            ),
            Ok(9 << 20)
        );
        for (text, bytes) in [("8 MiB", 8 << 20), ("2 GiB", 2 << 30), ("8388608", 8 << 20)] {
            assert_eq!(setting(&UNPACK_FLOOR, Some(text), ""), Ok(bytes), "{text}");
        }
        for text in ["7 MiB", "8388607", "3 GiB", "8 MB", "0", "-5", "wide"] {
            assert_eq!(
                setting(&UNPACK_FLOOR, Some(text), ""),
                Err(format!(
                    "BILBO_SESSION_UNPACK_FLOOR must be between 8 MiB and 2 GiB, got \"{text}\""
                )),
                "{text}"
            );
        }
        assert_eq!(
            setting(&UNPACK_FLOOR, None, "session.unpack_floor = \"7 MiB\""),
            Err("/c/config.toml:1: session.unpack_floor must be between 8 MiB and 2 GiB, got \"7 MiB\"".to_owned())
        );
    }

    #[test]
    fn the_unpack_ratio_is_64_to_4096_and_defaults_to_256() {
        assert_eq!(setting(&UNPACK_RATIO, None, ""), Ok(256));
        assert_eq!(setting(&UNPACK_RATIO, Some("64"), ""), Ok(64));
        assert_eq!(setting(&UNPACK_RATIO, Some("4096"), ""), Ok(4096));
        for text in ["63", "4097", "0", "-5", "wide", "64.5"] {
            assert_eq!(
                setting(&UNPACK_RATIO, Some(text), ""),
                Err(format!(
                    "BILBO_SESSION_UNPACK_RATIO must be a whole number from 64 to 4096, got \"{text}\""
                )),
                "{text}"
            );
        }
        assert_eq!(
            setting(&UNPACK_RATIO, None, "session.unpack_ratio = 4097"),
            Err("/c/config.toml:1: session.unpack_ratio must be a whole number from 64 to 4096, got 4097".to_owned())
        );
    }

    #[test]
    fn a_session_built_with_a_lower_floor_or_ratio_refuses_what_the_default_keeps() {
        let name = PathBuf::from(format!("{}.zst", rollout()));
        let first = CODEX
            .split_inclusive(|&byte| byte == b'\n')
            .next()
            .expect("a line");
        let mut feeds = first.to_vec();
        feeds.resize(first.len() + (1 << 20), b'\n');
        let feeds = zstd::bulk::compress(&feeds, 3).expect("packs");
        assert!(Session::default().unpack(&name, feeds.clone()).is_ok());
        assert_eq!(
            Session::new(8 << 20, 256).unpack(&name, feeds).err(),
            Some(Refusal(
                "would hold more than 8388608 bytes unpacked".to_owned()
            ))
        );

        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut noise = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            u8::try_from(state & 0xff).expect("a byte")
        };
        let mut hard = first.to_vec();
        hard.extend((0..256 << 10).map(|_| match noise() {
            b'\n' => b'x',
            byte => byte,
        }));
        hard.resize(hard.len() + 600_000, b'\n');
        let hard = zstd::bulk::compress(&hard, 3).expect("packs");
        assert!(
            Session::new(8 << 20, 256)
                .unpack(&name, hard.clone())
                .is_ok()
        );
        assert_eq!(
            Session::new(8 << 20, 64)
                .unpack(&name, hard)
                .err()
                .map(|refusal| refusal.0.starts_with("would hold more than ")),
            Some(true)
        );
    }

    const CLAUDE_FOLDERS: &[u8] =
        include_bytes!("../fixtures/7d1e4b9a-3c5f-4a2e-8b6d-0f9e1c2a3b4d.jsonl");
    const CODEX_FOLDERS: &[u8] = include_bytes!(
        "../fixtures/rollout-2026-11-08T10-00-00-019a6c4d-3e5f-7a71-9b2c-4d6e8fa0b1c3.jsonl"
    );
    const CODEX_LONG: &[u8] = include_bytes!(
        "../fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
    );
    const CLAUDE_LONG: &[u8] =
        include_bytes!("../fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl");

    fn labels_of(bytes: &[u8]) -> Vec<String> {
        use bilbo_kinds::reader::Reader;
        let records: Vec<&[u8]> = bytes.split_inclusive(|&byte| byte == b'\n').collect();
        Session::default().entry_labels(&records)
    }

    #[test]
    fn a_claude_session_gives_each_cwd_and_its_added_directories() {
        assert_eq!(
            labels_of(CLAUDE_FOLDERS),
            [
                "/home/me/src/garden",
                "/home/me/notes",
                "/home/me/src/shed",
                "/home/me/src/garden/beds",
                "/home/me/src/greenhouse",
            ]
        );
    }

    #[test]
    fn a_codex_session_gives_the_cwd_of_its_meta_and_its_turn_contexts() {
        assert_eq!(
            labels_of(CODEX_FOLDERS),
            ["/home/me/src/garden", "/home/me/src/shed"]
        );
    }

    #[test]
    fn the_older_fixtures_give_their_one_cwd_and_cursor_gives_none() {
        for bytes in [CLAUDE, CLAUDE_LONG, SUBAGENT, CODEX, CODEX_LONG] {
            assert_eq!(labels_of(bytes), ["/home/me/src/garden"]);
        }
        assert!(labels_of(CURSOR).is_empty());
        assert!(labels_of(NONE).is_empty());
        assert!(labels_of(b"").is_empty());
    }

    #[test]
    fn a_cursor_record_with_a_cwd_gives_it() {
        let line = br#"{"role":"user","cwd":"/home/me/src/garden","message":{"content":[{"type":"text","text":"Water the beds."}]}}"#;
        let bytes = [CURSOR, line.as_slice(), b"\n"].concat();
        let records = bytes.split_inclusive(|&byte| byte == b'\n');
        assert_eq!(
            crate::dialect::sniff(records),
            Some(crate::dialect::Dialect::Cursor)
        );
        assert_eq!(labels_of(&bytes), ["/home/me/src/garden"]);
    }

    #[test]
    fn a_record_that_is_no_json_object_or_whose_cwd_is_no_text_gives_nothing() {
        let bytes = [
            &b"{\"type\":\"user\",\"cwd\":12,\"message\":{\"content\":\"a\"}}\n"[..],
            b"{\"type\":\"user\",\"cwd\":[\"/a\"],\"message\":{\"content\":\"a\"}}\n",
            b"{\"type\":\"user\",\"cwd\":\"\",\"message\":{\"content\":\"a\"}}\n",
            b"{\"type\":\"user\",\"cwd\":null,\"message\":{\"content\":\"a\"}}\n",
            b"[\"/b\"]\n",
            b"7\n",
            b"{\"type\":\"user\",\"cwd\":\"/c\" broken\n",
            b"{\"type\":\"attachment\",\"attachment\":{\"type\":\"environment\",\"snapshot\":{\"additionalWorkingDirectories\":[3,\"\",null,\"/d\"]}}}\n",
            b"{\"type\":\"attachment\",\"attachment\":{\"type\":\"environment\",\"snapshot\":{\"additionalWorkingDirectories\":\"/e\"}}}\n",
        ]
        .concat();
        assert_eq!(labels_of(&bytes), ["/d"]);
        let codex = [
            &b"{\"type\":\"session_meta\",\"payload\":{\"cwd\":4}}\n"[..],
            b"{\"type\":\"turn_context\",\"payload\":{\"cwd\":\"\"}}\n",
            b"{\"type\":\"turn_context\",\"payload\":\"/f\"}\n",
            b"{\"type\":\"turn_context\",\"payload\":{}}\n",
        ]
        .concat();
        assert!(labels_of(&codex).is_empty());
    }

    #[test]
    fn a_folder_the_store_replaced_gives_no_label() {
        let long: String = "/ab".repeat(100);
        let replaced = bilbo_store::opaque::placeholder(&long).expect("a long folder is replaced");
        let claude = format!(
            "{{\"type\":\"user\",\"cwd\":\"{replaced}\",\"message\":{{\"content\":\"a\"}}}}\n\
             {{\"type\":\"user\",\"cwd\":\"/home/me/a\",\"message\":{{\"content\":\"b\"}}}}\n\
             {{\"type\":\"attachment\",\"attachment\":{{\"type\":\"environment\",\"snapshot\":{{\"additionalWorkingDirectories\":[\"{replaced}\",\"/home/me/b\"]}}}}}}\n"
        );
        assert_eq!(labels_of(claude.as_bytes()), ["/home/me/a", "/home/me/b"]);
        let codex = format!(
            "{{\"type\":\"turn_context\",\"payload\":{{\"cwd\":\"{replaced}\"}}}}\n\
             {{\"type\":\"turn_context\",\"payload\":{{\"cwd\":\"/home/me/c\"}}}}\n"
        );
        assert_eq!(labels_of(codex.as_bytes()), ["/home/me/c"]);
    }

    #[test]
    fn an_attachment_that_is_no_environment_gives_nothing() {
        let bytes = [
            &b"{\"type\":\"attachment\",\"attachment\":{\"type\":\"hook_success\",\"snapshot\":{\"additionalWorkingDirectories\":[\"/a\"]}}}\n"[..],
            b"{\"type\":\"user\",\"attachment\":{\"type\":\"environment\",\"snapshot\":{\"additionalWorkingDirectories\":[\"/b\"]}},\"message\":{\"content\":\"a\"}}\n",
            b"{\"type\":\"attachment\",\"attachment\":{\"snapshot\":{\"additionalWorkingDirectories\":[\"/c\"]}}}\n",
        ]
        .concat();
        assert!(labels_of(&bytes).is_empty());
    }
}
