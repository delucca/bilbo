use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::{fs, sync::LazyLock};

use bilbo_search::analyze;
use bilbo_search::bm25::Bm25;
use bilbo_search::index::{Extent, IndexError};
use bilbo_search::lookup::{Filter, Found, Lookup, Posting, Totals};
use bilbo_search::position::Position;
use bilbo_search::rank::{Query, Ranker};
use bilbo_store::entries::{Generation, RecordRange};
use bilbo_store::id::Id;
use serde_json::Value;

use super::{EXPECTED, rows, say, text, var};

const RANKS: &str = include_str!("ranks.jsonl");
const DOCS: &str = include_str!("rank-docs.jsonl");
const TOLERANCE: f64 = 1e-5;

struct Table {
    postings: HashMap<String, Vec<Posting>>,
    lengths: Vec<u32>,
}

impl Table {
    fn new(documents: &[Vec<String>]) -> Table {
        let mut postings: HashMap<String, Vec<Posting>> = HashMap::new();
        for (doc, terms) in documents.iter().enumerate() {
            let mut counts: HashMap<&str, u32> = HashMap::new();
            for term in terms {
                *counts.entry(term).or_default() += 1;
            }
            let length = u32::try_from(terms.len()).expect("a length");
            for (term, freq) in counts {
                postings.entry(term.to_owned()).or_default().push(Posting {
                    doc: u32::try_from(doc).expect("a document"),
                    freq,
                    length,
                });
            }
        }
        for list in postings.values_mut() {
            list.sort_by_key(|posting| posting.doc);
        }
        Table {
            postings,
            lengths: documents
                .iter()
                .map(|terms| u32::try_from(terms.len()).expect("a length"))
                .collect(),
        }
    }
}

fn entry(doc: u32) -> Id {
    let mut bytes = [0; 16];
    bytes[..4].copy_from_slice(&doc.to_be_bytes());
    Id::from_bytes(bytes)
}

fn doc_of(entry: Id) -> u32 {
    let bytes = entry.to_bytes();
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

impl Lookup for Table {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        Ok(None)
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        Ok(Totals {
            documents: self.lengths.len() as u64,
            entries: self.lengths.len() as u64,
            length: self.lengths.iter().map(|length| u64::from(*length)).sum(),
            bound: u32::try_from(self.lengths.len()).expect("a bound"),
        })
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        Ok(self.postings.get(term).cloned().unwrap_or_default())
    }

    fn holding(&self, _filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        Ok((0..self.lengths.len() as u32).collect())
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        Ok(docs
            .iter()
            .map(|doc| Found {
                entry: entry(*doc),
                kind: "session".to_owned(),
                extent: Extent {
                    generation: Generation(1),
                    records: RecordRange {
                        first: u64::from(*doc) + 1,
                        count: 1,
                    },
                    part: None,
                    date: 0,
                },
                digest: 0,
            })
            .collect())
    }
}

fn terms_of(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("terms")
        .iter()
        .map(|term| term.as_str().expect("a term").to_owned())
        .collect()
}

static CORPORA: LazyLock<HashMap<String, Table>> = LazyLock::new(|| {
    let mut documents: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for row in rows(EXPECTED) {
        if row.get("span").is_some() {
            documents
                .entry("sample".to_owned())
                .or_default()
                .push(terms_of(&row["terms"]));
        }
    }
    for row in rows(DOCS) {
        documents
            .entry(text(&row, "corpus"))
            .or_default()
            .push(terms_of(&row["terms"]));
    }
    documents
        .into_iter()
        .map(|(name, documents)| (name, Table::new(&documents)))
        .collect()
});

/// Document, bm25s's score, the bits of that score.
type Hit = (u32, f32, u32);

fn hits(row: &Value) -> Vec<Hit> {
    row["top"]
        .as_array()
        .expect("top")
        .iter()
        .map(|hit| {
            let bits = u32::try_from(hit[2].as_u64().expect("bits")).expect("32 bits");
            (
                u32::try_from(hit[0].as_u64().expect("a document")).expect("a document"),
                f32::from_bits(bits),
                bits,
            )
        })
        .collect()
}

fn bilbos(index: &dyn Lookup, query: &str, limit: usize) -> Vec<(u32, f32)> {
    Bm25 {
        index,
        k1: 1.2,
        b: 0.75,
        dropped: &[],
    }
    .rank(
        &Query {
            text: query.to_owned(),
            kinds: Vec::new(),
        },
        limit,
    )
    .expect("a ranking")
    .into_iter()
    .map(|candidate| (doc_of(candidate.found.entry), candidate.score))
    .collect()
}

fn ranked(row: &Value) -> (Vec<Hit>, Vec<(u32, f32)>) {
    let wanted = hits(row);
    let table = &CORPORA[&text(row, "corpus")];
    let found = bilbos(table, &text(row, "query"), wanted.len().max(10));
    if wanted.is_empty() {
        assert!(found.is_empty(), "{row}");
    }
    let limit = wanted.len();
    let mut found = found;
    if limit > 0 {
        found = bilbos(table, &text(row, "query"), limit);
    }
    (wanted, found)
}

#[test]
fn a_query_is_analyzed_as_bm25s_tokenizes_it() {
    let all = rows(RANKS);
    assert!(all.len() >= 50, "{}", all.len());
    for row in all {
        assert_eq!(
            analyze::query(&text(&row, "query"), &[]),
            terms_of(&row["terms"]),
            "{}",
            row["query"]
        );
    }
}

#[test]
fn bilbos_bm25_gives_bm25ss_scores_within_tolerance() {
    for row in rows(RANKS) {
        let (wanted, found) = ranked(&row);
        let docs = |list: &[u32]| list.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(
            docs(&found.iter().map(|hit| hit.0).collect::<Vec<_>>()),
            docs(&wanted.iter().map(|hit| hit.0).collect::<Vec<_>>()),
            "{}",
            row["query"]
        );
        for (doc, score, _) in &wanted {
            let (_, mine) = found.iter().find(|hit| hit.0 == *doc).expect("a hit");
            let relative = (f64::from(*mine) - f64::from(*score)).abs() / f64::from(*score);
            assert!(
                relative <= TOLERANCE,
                "{} document {doc}: {mine} against bm25s {score}",
                row["query"]
            );
        }
    }
}

#[test]
fn two_documents_compare_as_bm25s_compares_them() {
    let mut pairs = 0;
    for row in rows(RANKS) {
        let (wanted, found) = ranked(&row);
        let scores: HashMap<u32, f32> = found.into_iter().collect();
        for (at, first) in wanted.iter().enumerate() {
            if row["corpus"] != "sample" {
                assert_eq!(scores[&first.0].to_bits(), first.2, "{}", row["query"]);
            }
            for second in &wanted[at + 1..] {
                pairs += 1;
                assert_eq!(
                    scores[&first.0].total_cmp(&scores[&second.0]),
                    first.1.total_cmp(&second.1),
                    "{}: documents {} and {}",
                    row["query"],
                    first.0,
                    second.0
                );
            }
        }
    }
    assert!(pairs > 500, "{pairs}");
}

#[test]
fn bilbos_order_is_bm25ss_wherever_bm25s_separates_two_documents() {
    for row in rows(RANKS) {
        let (wanted, found) = ranked(&row);
        let mut at = 0;
        while at < wanted.len() {
            let run = wanted[at..]
                .iter()
                .take_while(|hit| hit.2 == wanted[at].2)
                .count();
            let docs = |list: &[u32]| list.iter().copied().collect::<BTreeSet<_>>();
            assert_eq!(
                docs(
                    &found[at..at + run]
                        .iter()
                        .map(|hit| hit.0)
                        .collect::<Vec<_>>()
                ),
                docs(
                    &wanted[at..at + run]
                        .iter()
                        .map(|hit| hit.0)
                        .collect::<Vec<_>>()
                ),
                "{} from place {at}",
                row["query"]
            );
            at += run;
        }
    }
}

/// `CONTRACT_STORE` is the unpacked eval store, `CONTRACT_WHOLE` the folder `regenerate.py --whole` wrote.
#[test]
#[ignore = "needs the unpacked eval store"]
fn the_bm25_arms_286_rankings_are_bilbos_up_to_exact_ties() {
    let store = PathBuf::from(var("CONTRACT_STORE"));
    let folder = PathBuf::from(var("CONTRACT_WHOLE"));
    let mut tables: HashMap<String, (Table, HashMap<u64, u32>)> = HashMap::new();
    let mut by_repo: HashMap<String, Vec<(u64, Vec<String>)>> = HashMap::new();
    for row in rows(&fs::read_to_string(store.join("chunks.jsonl")).expect("chunks.jsonl")) {
        by_repo.entry(text(&row, "repo")).or_default().push((
            row["id"].as_u64().expect("an id"),
            analyze::terms(&text(&row, "text")),
        ));
    }
    for (repo, chunks) in by_repo {
        let ids = chunks
            .iter()
            .zip(0..)
            .map(|((id, _), doc)| (*id, doc))
            .collect();
        let documents: Vec<Vec<String>> = chunks.into_iter().map(|(_, terms)| terms).collect();
        tables.insert(repo, (Table::new(&documents), ids));
    }
    let (mut lists, mut places) = (0, 0);
    let lines =
        rows(&fs::read_to_string(folder.join("ranks-whole.jsonl")).expect("ranks-whole.jsonl"));
    for row in &lines {
        let (table, ids) = &tables[&text(row, "repo")];
        let wanted = row["top"].as_array().expect("top");
        let theirs: HashMap<u32, f64> = wanted
            .iter()
            .map(|hit| {
                (
                    ids[&hit[0].as_u64().expect("an id")],
                    hit[1].as_f64().expect("a score"),
                )
            })
            .collect();
        let found = bilbos(table, &text(row, "query"), 50.min(wanted.len()));
        assert_eq!(found.len(), 50.min(wanted.len()), "{}", row["id"]);
        for (place, (doc, score)) in found.iter().enumerate() {
            let at = wanted[place][1].as_f64().expect("a score");
            let own = theirs.get(doc).unwrap_or_else(|| {
                panic!(
                    "{} {}: place {place} is not in bm25s's 200",
                    row["id"], row["form"]
                )
            });
            let close = |a: f64, b: f64| (a - b).abs() <= TOLERANCE * b.abs();
            assert!(
                close(*own, at) && close(f64::from(*score), at),
                "{} {}: place {place}: bm25s's {own} against its {at}, bilbo's {score}",
                row["id"],
                row["form"]
            );
            places += 1;
        }
        lists += 1;
    }
    say(&format!("ranks: {lists} lists, {places} places"));
    assert_eq!(lists, 286);
}
