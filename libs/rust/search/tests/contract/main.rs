//! `expected.jsonl` is what the retrieval eval's bm25 arm built for `sample.jsonl` and what bm25s gives
//! for every text here, written by `regenerate.py`, never by hand.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::sync::LazyLock;

use bilbo_kinds::reader::{Passage, Piece};
use bilbo_qwen::Qwen;
use bilbo_search::analyze;
use bilbo_search::chunk::{Chunk, Size, chunks};
use bilbo_store::entries::RecordRange;
use serde_json::Value;

const SAMPLE: &str = include_str!("sample.jsonl");
const EDGES: &str = include_str!("edges.jsonl");
const EXPECTED: &str = include_str!("expected.jsonl");
const STEMS: &[u8] = include_bytes!("stems.tsv.zst");
const EVAL: Size = Size {
    tokens: 512,
    overlap: 64,
};

static QWEN: LazyLock<Qwen> = LazyLock::new(|| Qwen::load().expect("the tokenizer loads"));

/// First and last line, part, tokens, text.
type Row = (u64, u64, Option<(u32, u32)>, usize, String);

fn rows(text: &str) -> Vec<Value> {
    text.split('\n')
        .filter(|row| !row.is_empty())
        .map(|row| serde_json::from_str(row).expect("a row"))
        .collect()
}

fn text(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is a string"))
        .to_owned()
}

fn whole(value: &Value) -> u64 {
    value.as_u64().expect("a whole number")
}

fn passages(span: &Value) -> Vec<Passage> {
    span["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|message| {
            let text = text(message, "text");
            Passage {
                range: RecordRange {
                    first: whole(&message["line"]),
                    count: 1,
                },
                pieces: message["heads"]
                    .as_array()
                    .expect("heads")
                    .iter()
                    .map(|head| Piece {
                        start: usize::try_from(whole(&head[0])).expect("an offset"),
                        label: head[1].as_str().expect("a label line").to_owned(),
                    })
                    .collect(),
                ends: vec![text.len()],
                text,
                at: None,
                outline: None,
            }
        })
        .collect()
}

fn found(chunks: Vec<Chunk>) -> Vec<Row> {
    chunks
        .into_iter()
        .map(|chunk| {
            let last = chunk.records.first + chunk.records.count - 1;
            (
                chunk.records.first,
                last,
                chunk.part,
                chunk.tokens,
                chunk.text,
            )
        })
        .collect()
}

fn wanted(row: &Value) -> Row {
    let part = row["part"].as_array().map(|part| {
        let number = |at: usize| u32::try_from(whole(&part[at])).expect("a part");
        (number(0), number(1))
    });
    (
        whole(&row["lines"][0]),
        whole(&row["lines"][1]),
        part,
        usize::try_from(whole(&row["tokens"])).expect("tokens"),
        text(row, "text"),
    )
}

#[expect(
    clippy::disallowed_macros,
    reason = "a contract test says which chunk differs, and the whole-store check its counts"
)]
fn say(line: &str) {
    eprintln!("{line}");
}

fn differing(name: &str, found: &[Row], wanted: &[Row]) -> usize {
    let differing = found.len().abs_diff(wanted.len())
        + found.iter().zip(wanted).filter(|(a, b)| a != b).count();
    if differing > 0 {
        let first = found.iter().zip(wanted).find(|(a, b)| a != b);
        say(&format!(
            "{name}: {differing} of {} chunks differ ({} made); the first: {first:#?}",
            wanted.len(),
            found.len()
        ));
    }
    differing
}

#[test]
fn a_sample_of_eval_spans_chunks_as_the_eval_store_has_them() {
    let expected = rows(EXPECTED);
    let mut made = 0;
    for (at, span) in rows(SAMPLE).iter().enumerate() {
        let file = text(span, "file");
        let wanted: Vec<Row> = expected
            .iter()
            .filter(|row| row["span"].as_u64() == Some(at as u64))
            .map(wanted)
            .collect();
        assert!(!wanted.is_empty(), "span {at} of {file}");
        let found = found(chunks(&passages(span), &*QWEN, EVAL).expect("a text it splits"));
        assert_eq!(differing(&file, &found, &wanted), 0, "span {at} of {file}");
        made += found.len();
    }
    assert!(made >= 60, "{made} chunks");
}

#[test]
fn the_sample_holds_every_shape_of_a_chunk() {
    let shapes: Vec<String> = rows(SAMPLE)
        .iter()
        .map(|span| text(span, "shape"))
        .collect();
    for shape in [
        "one message alone",
        "several messages packed",
        "plain windows",
        "packed, windows, packed",
        "a window edge inside a character",
        "a window that loses tokens to fit",
        "a window that loses tokens twice",
        "a window that starts in a later piece",
        "a compact summary",
        "a Cursor session",
    ] {
        assert!(shapes.iter().any(|found| found == shape), "{shape}");
    }
}

#[test]
fn every_text_has_the_terms_bm25s_gave_it() {
    let edges = rows(EDGES);
    let (mut chunks, mut by_hand, mut generated) = (0, 0, 0);
    for row in rows(EXPECTED) {
        let text = if let Some(edge) = row["edge"].as_u64() {
            by_hand += 1;
            text(&edges[usize::try_from(edge).expect("an edge")], "text")
        } else {
            if row.get("generated").is_some() {
                generated += 1;
            } else {
                chunks += 1;
            }
            text(&row, "text")
        };
        let wanted: Vec<&str> = row["terms"]
            .as_array()
            .expect("terms")
            .iter()
            .map(|term| term.as_str().expect("a term"))
            .collect();
        let found = analyze::terms(&text);
        if found != wanted {
            let at = found.iter().zip(&wanted).position(|(a, b)| a != b);
            let near = at.map(|at| (&found[at], wanted[at]));
            panic!(
                "{} terms, bm25s {}; the first that differs: {near:?} in {:?}",
                found.len(),
                wanted.len(),
                text.chars().take(80).collect::<String>()
            );
        }
    }
    assert_eq!(by_hand, edges.len());
    assert!(chunks >= 60 && generated == 2, "{chunks} {generated}");
}

#[expect(
    clippy::disallowed_methods,
    reason = "the whole-store checks are told their files by the environment"
)]
fn var(name: &str) -> OsString {
    std::env::var_os(name).unwrap_or_else(|| panic!("{name}"))
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// `CONTRACT_STORE` is the unpacked eval store, `CONTRACT_WHOLE` the folder `regenerate.py --whole` wrote.
#[test]
#[ignore = "needs the unpacked eval store"]
fn the_whole_corpus_chunks_as_the_eval_store_has_it() {
    let store = std::path::PathBuf::from(var("CONTRACT_STORE"));
    let folder = std::path::PathBuf::from(var("CONTRACT_WHOLE"));
    let mut expected: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for row in rows(&fs::read_to_string(store.join("chunks.jsonl")).expect("chunks.jsonl")) {
        let file = format!("{}/{}", text(&row, "repo"), text(&row, "session"));
        expected.entry(file).or_default().push(wanted(&row));
    }
    let (mut sessions, mut all, mut differ) = (0, 0, 0);
    for span in rows(&fs::read_to_string(folder.join("messages.jsonl")).expect("messages.jsonl")) {
        let file = text(&span, "file");
        let wanted = expected.remove(&file).unwrap_or_default();
        differ += differing(
            &file,
            &found(chunks(&passages(&span), &*QWEN, EVAL).expect("a text it splits")),
            &wanted,
        );
        all += wanted.len();
        sessions += 1;
    }
    say(&format!(
        "corpus: {sessions} sessions, {all} chunks, {differ} differ, {} sessions never rendered",
        expected.len()
    ));
    assert!(all > 100_000 && expected.is_empty());
    assert_eq!(differ, 0);
}

#[test]
#[ignore = "needs the unpacked eval store"]
fn every_chunk_of_the_eval_store_has_the_terms_bm25s_gave_it() {
    let store = std::path::PathBuf::from(var("CONTRACT_STORE"));
    let folder = std::path::PathBuf::from(var("CONTRACT_WHOLE"));
    let digests: BTreeMap<String, (String, String)> =
        fs::read_to_string(folder.join("digests.tsv"))
            .expect("digests.tsv")
            .lines()
            .map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                (
                    fields[0].to_owned(),
                    (fields[1].to_owned(), fields[2].to_owned()),
                )
            })
            .collect();
    let (mut seen, mut differ) = (0, 0);
    for row in rows(&fs::read_to_string(store.join("chunks.jsonl")).expect("chunks.jsonl")) {
        let terms = analyze::terms(&text(&row, "text"));
        let (count, digest) = &digests[&row["id"].to_string()];
        seen += 1;
        if terms.len().to_string() != *count
            || format!("{:016x}", fnv(terms.join("\n").as_bytes())) != *digest
        {
            differ += 1;
            if differ < 10 {
                say(&format!(
                    "terms: chunk {} has {} terms, bm25s {count}",
                    row["id"],
                    terms.len()
                ));
            }
        }
    }
    say(&format!("chunks {seen}, terms differ {differ}"));
    assert!(seen > 100_000);
    assert_eq!(differ, 0);
}

#[test]
fn every_word_of_the_eval_store_stems_as_pystemmer_stemmed_it() {
    let table = zstd::bulk::decompress(STEMS, 8 << 20).expect("stems.tsv.zst unpacks");
    let table = String::from_utf8(table).expect("utf-8");
    let (mut rows, mut differ) = (0, Vec::new());
    for line in table.lines() {
        let mut fields = line.split('\t');
        let (word, cut, suffix) = (
            fields.next().expect("a word"),
            fields.next().expect("a cut"),
            fields.next().expect("a suffix"),
        );
        assert!(fields.next().is_none(), "{line:?}");
        let keep = word
            .chars()
            .count()
            .checked_sub(cut.parse::<usize>().expect("a count"))
            .expect("a cut within the word");
        let stem: String = word.chars().take(keep).chain(suffix.chars()).collect();
        rows += 1;
        if analyze::stem(word) != stem {
            differ.push((word.to_owned(), stem));
        }
    }
    if !differ.is_empty() {
        say(&format!(
            "{} of {rows} words stem differently; the first: {:?}",
            differ.len(),
            &differ[..differ.len().min(10)]
        ));
    }
    assert!(rows >= 180_000, "{rows} rows");
    assert!(differ.is_empty());
}
