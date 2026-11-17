use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use bilbo_search::chunk::at;
use bilbo_store::entries::RecordRange;
use serde_json::Value;

use super::{EVAL, EXPECTED, QWEN, SAMPLE, passages, rows, say, text, var, wanted};

fn coordinates(row: &Value) -> (RecordRange, Option<(u32, u32)>, usize, String) {
    let (first, last, part, tokens, text) = wanted(row);
    (
        RecordRange {
            first,
            count: last - first + 1,
        },
        part,
        tokens,
        text,
    )
}

#[test]
fn every_committed_chunk_is_recomputed_from_its_coordinates() {
    let spans = rows(SAMPLE);
    let mut recomputed = 0;
    for row in rows(EXPECTED) {
        let Some(span) = row["span"].as_u64() else {
            continue;
        };
        let passages = passages(&spans[usize::try_from(span).expect("a span")]);
        let (records, part, tokens, text) = coordinates(&row);
        let chunk = at(&passages, &*QWEN, EVAL, records, part)
            .expect("a text it splits")
            .unwrap_or_else(|| {
                panic!("span {span} {:?} {:?}: no chunk", row["lines"], row["part"])
            });
        assert_eq!(chunk.text, text, "span {span} {:?}", row["lines"]);
        assert_eq!(chunk.tokens, tokens, "span {span} {:?}", row["lines"]);
        recomputed += 1;
    }
    assert!(recomputed >= 60, "{recomputed}");
}

#[test]
#[ignore = "needs CONTRACT_STORE, the unpacked eval store, and CONTRACT_WHOLE, the folder regenerate.py --whole wrote"]
fn every_chunk_of_the_eval_store_is_recomputed_from_its_coordinates() {
    let store = PathBuf::from(var("CONTRACT_STORE"));
    let folder = PathBuf::from(var("CONTRACT_WHOLE"));
    let mut expected: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in rows(&fs::read_to_string(store.join("chunks.jsonl")).expect("chunks.jsonl")) {
        let file = format!("{}/{}", text(&row, "repo"), text(&row, "session"));
        expected.entry(file).or_default().push(row);
    }
    let (mut all, mut differ) = (0, 0);
    for span in rows(&fs::read_to_string(folder.join("messages.jsonl")).expect("messages.jsonl")) {
        let file = text(&span, "file");
        let passages = passages(&span);
        for row in expected.remove(&file).unwrap_or_default() {
            all += 1;
            let (records, part, _, wanted) = coordinates(&row);
            let found = at(&passages, &*QWEN, EVAL, records, part).expect("a text it splits");
            let same = found.as_ref().is_some_and(|chunk| chunk.text == wanted);
            if !same {
                differ += 1;
                if differ < 10 {
                    say(&format!(
                        "{file} {:?} {:?}: not recomputed",
                        row["lines"], row["part"]
                    ));
                }
            }
        }
    }
    say(&format!(
        "coordinates: {all} chunks, {differ} not recomputed, {} sessions never rendered",
        expected.len()
    ));
    assert!(all > 100_000 && expected.is_empty());
    assert_eq!(differ, 0);
}
