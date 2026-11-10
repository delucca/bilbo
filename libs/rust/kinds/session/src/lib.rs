mod claude;
mod codex;
mod dialect;
mod json;
mod render;

use std::path::{Path, PathBuf};

use bilbo_kinds::files::{Files, Node};
use bilbo_kinds::kind::{Claims, Kind, Proposal, Refusal};
use bilbo_kinds::reader::{Passage, Reader};
use bilbo_store::id::Id;

use crate::dialect::Dialect;

const NAME: &str = "session";

/// Claude Code's and Codex's transcripts, and Cursor's Claude-shaped ones: one record per line.
#[derive(Debug, Clone, Copy, Default)]
pub struct Session;

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
            Err(error) => walk.refuse(target, &error),
        }
        walk.sessions.append(&mut walk.subagents);
        Claims {
            files: walk.sessions,
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

    fn propose(&self, path: &Path, _bytes: &[u8], kept: &dyn Fn(&Path) -> Option<Id>) -> Proposal {
        Proposal {
            id: None,
            parent: claude::session_of(path).and_then(|session| kept(&session)),
        }
    }
}

#[derive(Default)]
struct Walk {
    sessions: Vec<PathBuf>,
    subagents: Vec<PathBuf>,
    refused: Vec<(PathBuf, String)>,
}

impl Walk {
    /// Ends in a UUID and `.jsonl`: `<uuid>.jsonl`, `rollout-<time>-<uuid>.jsonl`,
    /// `<time>--<uuid>.jsonl`.
    fn file(&mut self, path: &Path) {
        if claude::session_of(path).is_some() {
            self.subagents.push(path.to_path_buf());
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".jsonl"))
            .is_some_and(|stem| {
                let stem = stem.as_bytes();
                stem.len() >= 36 && is_uuid(&stem[stem.len() - 36..])
            })
        {
            self.sessions.push(path.to_path_buf());
        }
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
                Err(error) => self.refuse(&path, &error),
            }
        }
    }

    fn refuse(&mut self, path: &Path, error: &std::io::Error) {
        self.refused
            .push((path.to_path_buf(), format!("cannot read: {error}")));
    }
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
    use std::path::{Path, PathBuf};

    use bilbo_kinds::files::{Memory, Node};
    use bilbo_kinds::kind::{Kind, Proposal, Refusal};
    use bilbo_kinds::reader::Passage;
    use bilbo_store::id::Id;

    use super::{Session, is_uuid};

    const CLAUDE: &[u8] = include_bytes!("../fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl");
    const SUBAGENT: &[u8] = include_bytes!("../fixtures/agent-ad63c2b3c9e7a4b3d.jsonl");
    const META: &[u8] = include_bytes!("../fixtures/agent-ad63c2b3c9e7a4b3d.meta.json");
    const CODEX: &[u8] = include_bytes!(
        "../fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
    );
    const CURSOR: &[u8] = include_bytes!("../fixtures/cursor-session.jsonl");
    const NONE: &[u8] = include_bytes!("../fixtures/not-a-session.jsonl");
    const NOT_UTF8: &[u8] = include_bytes!("../fixtures/not-utf8.jsonl");

    const MAIN: &str = "5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03";
    const OTHER: &str = "0e1d2c3b-4a59-4687-b6a5-a4b3c2d1e0f9";

    fn claimed(files: &Memory, target: &str) -> Vec<String> {
        Session
            .claims(files, Path::new(target))
            .files
            .iter()
            .map(|path| path.to_str().unwrap_or_default().to_owned())
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
        assert!(Session.claims(&projects(), Path::new(p)).refused.is_empty());
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
        let claims = Session.claims(&files, Path::new("/h/.claude/projects"));
        assert_eq!(claims.files.len(), 4);
        assert_eq!(claims.refused.len(), 1);
        assert_eq!(
            claims.refused[0].0,
            Path::new("/h/.claude/projects/-h-locked")
        );
        assert!(claims.refused[0].1.starts_with("cannot read: "));
        assert_eq!(
            Session
                .claims(&Memory::new(), Path::new("/nope"))
                .refused
                .len(),
            1
        );
    }

    #[test]
    fn each_line_is_a_record_and_the_records_join_back_to_the_file() {
        for bytes in [CLAUDE, SUBAGENT, CODEX, CURSOR] {
            let ends = Session.split(bytes).expect("a session");
            assert_eq!(ends.last().copied(), Some(bytes.len()));
            assert_eq!(ends.len(), bytes.iter().filter(|&&b| b == b'\n').count());
        }
        assert_eq!(Session.split(b""), Ok(vec![]));
    }

    /// A departure from the eval's render, which read a lone carriage return as a line end.
    #[test]
    fn a_lone_carriage_return_does_not_end_a_record() {
        use bilbo_kinds::reader::Reader;
        let entry =
            |text: &str| format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{text}\"}}}}");
        let bytes = format!("{}\r{}\n{}\r\n", entry("a"), entry("b"), entry("c")).into_bytes();
        let ends = Session.split(&bytes).expect("a session");
        assert_eq!(ends.len(), 2);
        let records: Vec<&[u8]> = bytes.split_inclusive(|&byte| byte == b'\n').collect();
        let read: Vec<(u64, String)> = Session
            .passages(&records)
            .into_iter()
            .map(|passage| (passage.range.first, passage.text))
            .collect();
        assert_eq!(read, [(2, "user: c".to_owned())]);
    }

    #[test]
    fn a_line_still_being_written_is_left_for_a_later_run() {
        let grown = [CURSOR, b"{\"role\":\"user\",\"mess"].concat();
        assert_eq!(Session.split(&grown), Session.split(CURSOR));
        assert_eq!(Session.split(b"{\"role\":\"us"), Ok(vec![]));
        let cut_char = [
            CURSOR,
            "{\"role\":\"é".as_bytes().split_last().expect("bytes").1,
        ]
        .concat();
        assert_eq!(Session.split(&cut_char), Session.split(CURSOR));
    }

    #[test]
    fn a_file_that_is_not_utf8_or_not_a_session_is_refused() {
        assert_eq!(
            Session.split(NOT_UTF8),
            Err(Refusal(format!(
                "not UTF-8 (byte {})",
                NOT_UTF8
                    .iter()
                    .position(|&b| b == 0xff)
                    .expect("a bad byte")
            )))
        );
        assert_eq!(
            Session.split(NONE),
            Err(Refusal(
                "no line is a Claude Code, Codex or Cursor record".to_owned()
            ))
        );
        let one_bad_line = [b"not json\n", CODEX].concat();
        assert_eq!(
            Session.split(&one_bad_line).map(|ends| ends.len()),
            Ok(1 + CODEX.iter().filter(|&&b| b == b'\n').count())
        );
    }

    #[test]
    fn a_subagent_proposes_the_entry_kept_for_its_session_as_parent() {
        let parent = Id::parse("01JC5Q3V7XKM4P8R2T6W9YB0DE").expect("id");
        let asked = RefCell::new(Vec::new());
        let kept = |path: &Path| {
            asked.borrow_mut().push(path.to_path_buf());
            Some(parent)
        };
        let p = "/h/.claude/projects/-h-src-garden";
        let subagent = format!("{p}/{MAIN}/subagents/agent-ad63c2b3c9e7a4b3d.jsonl");
        assert_eq!(
            Session.propose(Path::new(&subagent), SUBAGENT, &kept),
            Proposal {
                id: None,
                parent: Some(parent)
            }
        );
        assert_eq!(
            *asked.borrow(),
            [PathBuf::from(format!("{p}/{MAIN}.jsonl"))]
        );
        let main = format!("{p}/{MAIN}.jsonl");
        assert_eq!(
            Session.propose(Path::new(&main), CLAUDE, &kept),
            Proposal::default()
        );
        assert_eq!(
            Session.propose(Path::new(&subagent), SUBAGENT, &|_| None),
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
            Session
                .passages(&records)
                .first()
                .map(|passage| (passage.range.first, passage.text.clone()))
        };
        assert_eq!(Reader::kind(&Session), Session.name());
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
            assert!(Session.split(bytes).is_ok(), "the file is claimed");
            Session
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

    /// xorshift64*, from a fixed seed: the same cases on every run.
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

        /// A line with a few bytes changed, dropped, added or repeated, or cut short.
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

    /// What `Reader` promises of any records: passages in record order, none sharing a record,
    /// all inside the generation, their offsets rising char boundaries of their text.
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
                Session.passages(&records),
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

    #[test]
    fn the_kind_is_named_session_with_its_summary() {
        assert_eq!(Session.name(), "session");
        assert_eq!(
            Session.summary(),
            "Claude Code and Codex sessions, one record per JSONL line"
        );
    }
}
