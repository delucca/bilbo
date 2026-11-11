//! `expected.jsonl` is what the eval's bm25 `render_session` printed for `edges.jsonl` and `sample.jsonl`,
//! written by `regenerate.py`, never by hand.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use bilbo_kinds::reader::Reader;
use bilbo_session::Session;
use serde_json::Value;

const EDGES: &[u8] = include_bytes!("edges.jsonl");
const SAMPLE: &str = include_str!("sample.jsonl");
const EXPECTED: &str = include_str!("expected.jsonl");

type Message = (u64, String, Vec<(u64, String)>);

fn read(session: &[u8]) -> Vec<Message> {
    let records: Vec<&[u8]> = session.split_inclusive(|byte| *byte == b'\n').collect();
    Session
        .passages(&records)
        .into_iter()
        .map(|passage| {
            assert_eq!(passage.range.count, 1, "a message is one record");
            let heads = passage
                .pieces
                .into_iter()
                .map(|piece| (piece.start as u64, piece.label))
                .collect();
            (passage.range.first, passage.text, heads)
        })
        .collect()
}

fn text(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is a string in {value}"))
        .to_owned()
}

fn expected(rows: &str) -> BTreeMap<String, Vec<Message>> {
    let mut sessions: BTreeMap<String, Vec<Message>> = BTreeMap::new();
    for row in rows.lines() {
        let row: Value = serde_json::from_str(row).expect("a row of expected.jsonl");
        let heads = row["heads"]
            .as_array()
            .expect("heads")
            .iter()
            .map(|head| {
                let label = head[1].as_str().expect("a label line").to_owned();
                (head[0].as_u64().expect("an offset"), label)
            })
            .collect();
        let line = row["line"].as_u64().expect("a line");
        sessions
            .entry(text(&row, "file"))
            .or_default()
            .push((line, text(&row, "text"), heads));
    }
    sessions
}

#[expect(
    clippy::disallowed_macros,
    reason = "a contract test says which message differs, and the whole-corpus check its counts"
)]
fn say(line: &str) {
    eprintln!("{line}");
}

fn compare(name: &str, found: &[Message], wanted: &[Message]) -> usize {
    let mut differing = found.len().abs_diff(wanted.len());
    for (found, wanted) in found.iter().zip(wanted) {
        if found != wanted {
            differing += 1;
        }
    }
    if differing > 0 {
        let first = found
            .iter()
            .zip(wanted)
            .find(|(found, wanted)| found != wanted);
        say(&format!(
            "{name}: {differing} of {} messages differ ({} read); the first: {first:#?}",
            wanted.len(),
            found.len()
        ));
    }
    differing
}

const FIXTURES: &[(&str, &[u8])] = &[
    (
        "claude",
        include_bytes!("../../fixtures/5c3a9e1f-0b7d-4c2e-9a41-7f2d8e6b1c03.jsonl"),
    ),
    (
        "claude, long",
        include_bytes!("../../fixtures/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl"),
    ),
    (
        "subagent",
        include_bytes!("../../fixtures/agent-ad63c2b3c9e7a4b3d.jsonl"),
    ),
    (
        "cursor",
        include_bytes!("../../fixtures/cursor-session.jsonl"),
    ),
    (
        "codex",
        include_bytes!(
            "../../fixtures/rollout-2026-11-06T09-10-00-019a4f2e-7c1d-7b3a-9e55-2d8c41f0a6b7.jsonl"
        ),
    ),
    (
        "codex, long",
        include_bytes!(
            "../../fixtures/rollout-2026-11-07T08-00-00-019a5b3c-2d4e-7f60-8a1b-3c5d7e9f0a2b.jsonl"
        ),
    ),
];

#[test]
fn appending_records_changes_no_existing_passage() {
    assert!(Session.appends_only());
    let mut cuts = 0;
    for (name, bytes) in FIXTURES.iter().chain(&[("edges", EDGES)]) {
        let records: Vec<&[u8]> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
        let whole = Session.passages(&records);
        for cut in 0..=records.len() {
            let part = Session.passages(&records[..cut]);
            assert!(
                part.len() <= whole.len() && whole[..part.len()] == part[..],
                "{name}: the passages of the first {cut} records are not the first passages of the whole"
            );
            cuts += 1;
        }
    }
    assert!(cuts > 100, "{cuts} cuts");
}

#[test]
fn every_branch_the_corpus_does_not_reach_reads_as_build_py_printed_it() {
    let expected = expected(EXPECTED);
    let wanted = &expected["edges"];
    assert!(wanted.len() > 100, "the edges are in expected.jsonl");
    assert_eq!(compare("edges", &read(EDGES), wanted), 0);
}

#[test]
fn a_sample_of_eval_sessions_reads_as_build_py_printed_it() {
    let expected = expected(EXPECTED);
    let mut sessions = 0;
    let mut messages = 0;
    for row in SAMPLE.lines() {
        let row: Value = serde_json::from_str(row).expect("a row of sample.jsonl");
        let name = text(&row, "file");
        let lines: String = row["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .map(|line| line.as_str().expect("a line"))
            .collect();
        let wanted = expected.get(&name).cloned().unwrap_or_default();
        assert_eq!(compare(&name, &read(lines.as_bytes()), &wanted), 0);
        sessions += 1;
        messages += wanted.len();
    }
    assert!(
        sessions >= 50 && messages >= 40,
        "{sessions} sessions, {messages} messages"
    );
    assert_eq!(
        expected.len(),
        expected
            .keys()
            .filter(|name| *name == "edges" || SAMPLE.contains(name.as_str()))
            .count(),
        "every expected session is in the sample"
    );
}

#[test]
fn the_sample_holds_cursors_entries_and_every_kind_of_piece() {
    let expected = expected(EXPECTED);
    let heads: Vec<&str> = expected
        .iter()
        .filter(|(name, _)| *name != "edges")
        .flat_map(|(_, messages)| messages)
        .flat_map(|(_, _, heads)| heads)
        .map(|(_, label)| label.as_str())
        .collect();
    for start in [
        "user:",
        "assistant:",
        "tool_use ",
        "tool_result ",
        "tool_error ",
        "compact summary:",
    ] {
        assert!(heads.iter().any(|head| head.starts_with(start)), "{start}");
    }
    let cursor = SAMPLE.lines().any(|row| {
        let row: Value = serde_json::from_str(row).expect("a row");
        row["lines"].as_array().expect("lines").iter().any(|line| {
            let entry: Value =
                serde_json::from_str(line.as_str().expect("a line")).unwrap_or_default();
            entry.get("type").is_none() && entry.get("role").is_some()
        })
    });
    assert!(cursor, "a Cursor session is in the sample");
}

#[expect(
    clippy::disallowed_methods,
    reason = "the whole-corpus check is told its folders by the environment"
)]
fn var(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// `CONTRACT_CORPUS` is the unpacked eval corpus, `CONTRACT_EXPECTED` what `regenerate.py --whole` wrote. Prints counts only.
#[test]
#[ignore = "needs the unpacked eval corpus"]
fn the_whole_corpus_reads_as_build_py_printed_it() {
    let corpus = PathBuf::from(var("CONTRACT_CORPUS").expect("CONTRACT_CORPUS"));
    let rows = fs::read_to_string(var("CONTRACT_EXPECTED").expect("CONTRACT_EXPECTED"))
        .expect("read CONTRACT_EXPECTED");
    let expected = expected(&rows);
    let (mut messages, mut differing, mut sessions) = (0, 0, 0);
    for (name, wanted) in &expected {
        let session = fs::read(corpus.join(name)).expect("read a session");
        differing += compare(name, &read(&session), wanted);
        messages += wanted.len();
        sessions += 1;
    }
    say(&format!(
        "corpus: {sessions} sessions, {messages} messages, {differing} differ"
    ));
    assert!(sessions > 0, "the corpus is empty");
    assert_eq!(differing, 0);
}
