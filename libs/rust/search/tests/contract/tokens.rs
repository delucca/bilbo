//! The chunks of `expected.jsonl` carry the token count llama.cpp gave each text when the retrieval
//! eval's bm25 arm built its store.

use std::collections::BTreeMap;
use std::fs;

use bilbo_entries::tokens::Tokens;

use super::{EXPECTED, QWEN, fnv, rows, say, var};

fn joins(text: &str, ends: &[usize]) -> bool {
    ends.last().copied().unwrap_or(0) == text.len()
        && ends.windows(2).all(|pair| pair[0] <= pair[1])
}

#[test]
fn every_sample_chunk_has_the_tokens_llama_counted() {
    let mut chunks = 0;
    for row in rows(EXPECTED) {
        let (Some(text), Some(tokens)) = (row["text"].as_str(), row["tokens"].as_u64()) else {
            continue;
        };
        let ends = QWEN.ends(text);
        assert_eq!(ends.len() as u64, tokens, "{:?}", row["lines"]);
        assert_eq!(QWEN.count(text), ends.len());
        assert!(joins(text, &ends), "{:?}", row["lines"]);
        chunks += 1;
    }
    assert!(chunks >= 60, "{chunks}");
}

#[test]
fn every_text_of_the_fixture_joins_back_from_its_tokens() {
    let mut inside = 0;
    for row in rows(EXPECTED) {
        let Some(text) = row["text"].as_str() else {
            continue;
        };
        let ends = QWEN.ends(text);
        assert!(joins(text, &ends));
        inside += ends
            .iter()
            .filter(|end| !text.is_char_boundary(**end))
            .count();
    }
    assert!(
        inside > 0,
        "no token of the fixture ends inside a character"
    );
}

/// `CONTRACT_STORE` is the unpacked eval store, `CONTRACT_WHOLE` the folder `regenerate.py --whole` wrote.
#[test]
#[ignore = "needs the unpacked eval store"]
fn every_chunk_of_the_eval_store_has_the_tokens_llama_counted() {
    let store = std::path::PathBuf::from(var("CONTRACT_STORE"));
    let folder = std::path::PathBuf::from(var("CONTRACT_WHOLE"));
    let digests: BTreeMap<String, String> = fs::read_to_string(folder.join("digests.tsv"))
        .expect("digests.tsv")
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            (fields[0].to_owned(), fields[4].to_owned())
        })
        .collect();
    let (mut seen, mut counts, mut pieces) = (0, 0, 0);
    for row in rows(&fs::read_to_string(store.join("chunks.jsonl")).expect("chunks.jsonl")) {
        let text = row["text"].as_str().expect("text");
        let ends = QWEN.ends(text);
        seen += 1;
        if ends.len() as u64 != row["tokens"].as_u64().expect("tokens") {
            counts += 1;
        }
        let joined = ends
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if format!("{:016x}", fnv(joined.as_bytes())) != digests[&row["id"].to_string()] {
            pieces += 1;
        }
    }
    say(&format!(
        "chunks {seen}, counts differ {counts}, ends differ {pieces}"
    ));
    assert!(seen > 100_000);
    assert_eq!((counts, pieces), (0, 0));
}
