use std::cmp::Ordering;
use std::collections::HashMap;

use crate::analyze::{self, Omit};
use crate::index::IndexError;
use crate::lookup::{Filter, Lookup};
use crate::rank::{Candidate, Query, Ranked, Ranker, order};

pub struct Bm25<'a> {
    pub index: &'a dyn Lookup,
    pub k1: f64,
    pub b: f64,
    pub omit: Omit<'a>,
    pub batch: usize,
}

impl Bm25<'_> {
    fn read(&self, docs: &[u32], scores: &[f32]) -> Result<Vec<Candidate>, IndexError> {
        let mut read = Vec::with_capacity(docs.len());
        for batch in docs.chunks(self.batch) {
            let found = self.index.found(batch)?;
            read.extend(found.into_iter().zip(batch).map(|(found, doc)| Candidate {
                found,
                score: scores[*doc as usize],
            }));
        }
        Ok(read)
    }

    fn best(
        &self,
        mut matched: Vec<(u32, f32)>,
        limit: usize,
        scores: &[f32],
    ) -> Result<Vec<Candidate>, IndexError> {
        let mut best = if matched.len() <= limit {
            let docs: Vec<u32> = matched.iter().map(|(doc, _)| *doc).collect();
            self.read(&docs, scores)?
        } else {
            matched.select_nth_unstable_by(limit - 1, |a, b| b.1.total_cmp(&a.1));
            let last = matched[limit - 1].1;
            let mut above = Vec::new();
            let mut tied = Vec::new();
            for (doc, score) in &matched {
                match score.total_cmp(&last) {
                    Ordering::Greater => above.push(*doc),
                    Ordering::Equal => tied.push(*doc),
                    Ordering::Less => {}
                }
            }
            let need = limit - above.len();
            let mut kept = self.read(&above, scores)?;
            let mut ties: Vec<Candidate> = Vec::new();
            for batch in tied.chunks(self.batch) {
                ties.extend(self.read(batch, scores)?);
                ties.sort_by(order);
                ties.truncate(need);
            }
            kept.extend(ties);
            kept
        };
        best.sort_by(order);
        best.truncate(limit);
        Ok(best)
    }
}

impl Ranker for Bm25<'_> {
    fn rank(&self, query: &Query, limit: usize) -> Result<Ranked, IndexError> {
        let terms = analyze::query(&query.text, &self.omit);
        let nobody = query.entries.as_ref().is_some_and(Vec::is_empty);
        if terms.is_empty() || limit == 0 || nobody {
            return Ok(Ranked::default());
        }
        let totals = self.index.totals()?;
        if totals.documents == 0 {
            return Ok(Ranked::default());
        }
        let documents = totals.documents as f64;
        let average = totals.length as f64 / documents;
        let mut weighed: HashMap<&str, Vec<(u32, f32)>> = HashMap::new();
        for term in &terms {
            if weighed.contains_key(term.as_str()) {
                continue;
            }
            let postings = self.index.postings(term)?;
            let frequency = postings.len() as f64;
            let idf = (1.0 + (documents - frequency + 0.5) / (frequency + 0.5)).ln() as f32;
            let parts = postings
                .iter()
                .map(|posting| {
                    let freq = f64::from(posting.freq);
                    let denominator = self.k1
                        * ((1.0 - self.b) + ((self.b * f64::from(posting.length)) / average))
                        + freq;
                    (posting.doc, (f64::from(idf) * (freq / denominator)) as f32)
                })
                .collect();
            weighed.insert(term, parts);
        }
        let mut scores = vec![0.0_f32; totals.bound as usize];
        for term in &terms {
            for (doc, part) in &weighed[term.as_str()] {
                let score = scores
                    .get_mut(*doc as usize)
                    .ok_or(IndexError::Inconsistent(
                        "a posting names a document outside the index",
                    ))?;
                *score += part;
            }
        }
        let allowed = if query.kinds.is_empty() {
            None
        } else {
            let mut allowed = vec![false; scores.len()];
            for kind in &query.kinds {
                for doc in self.index.holding(&Filter::Kind(kind))? {
                    *allowed
                        .get_mut(doc as usize)
                        .ok_or(IndexError::Inconsistent(
                            "a kind names a document outside the index",
                        ))? = true;
                }
            }
            Some(allowed)
        };
        let matched: Vec<(u32, f32)> = (0..totals.bound)
            .filter(|doc| {
                scores[*doc as usize] > 0.0
                    && allowed
                        .as_ref()
                        .is_none_or(|allowed| allowed[*doc as usize])
            })
            .map(|doc| (doc, scores[doc as usize]))
            .collect();
        let Some(ids) = &query.entries else {
            let candidates = self.best(matched, limit, &scores)?;
            return Ok(Ranked {
                candidates,
                unscoped: Vec::new(),
            });
        };
        let mut held = vec![false; scores.len()];
        for doc in self.index.holding(&Filter::Entries(ids))? {
            *held.get_mut(doc as usize).ok_or(IndexError::Inconsistent(
                "an entry names a document outside the index",
            ))? = true;
        }
        let scoped = matched
            .iter()
            .copied()
            .filter(|(doc, _)| held[*doc as usize])
            .collect();
        Ok(Ranked {
            candidates: self.best(scoped, limit, &scores)?,
            unscoped: self.best(matched, limit, &scores)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::analyze::{Language, Omit};
    use crate::index::IndexError;
    use crate::rank::{Candidate, Query, Ranker};
    use crate::testing::{Says, Table, id};

    use super::Bm25;

    fn query(text: &str, kinds: &[&str]) -> Query {
        Query {
            text: text.to_owned(),
            kinds: kinds.iter().map(|kind| (*kind).to_owned()).collect(),
            entries: None,
        }
    }

    fn scoped(text: &str, kinds: &[&str], entries: &[u8]) -> Query {
        Query {
            entries: Some(entries.iter().map(|n| id(*n)).collect()),
            ..query(text, kinds)
        }
    }

    fn bm25(table: &Table) -> Bm25<'_> {
        Bm25 {
            index: table,
            k1: 1.2,
            b: 0.75,
            omit: Omit::default(),
            batch: 4096,
        }
    }

    fn entries_of(list: &[Candidate]) -> Vec<(u8, u64)> {
        list.iter()
            .map(|c| (c.found.entry.to_bytes()[0], c.found.extent.records.first))
            .collect()
    }

    fn ranked(table: &Table, text: &str, kinds: &[&str], limit: usize) -> Vec<(u8, u64, f32)> {
        let bm25 = Bm25 {
            index: table,
            k1: 1.2,
            b: 0.75,
            omit: Omit::default(),
            batch: 4096,
        };
        bm25.rank(&query(text, kinds), limit)
            .expect("a ranking")
            .candidates
            .into_iter()
            .map(|candidate| {
                let entry = (1..=255)
                    .find(|n| id(*n) == candidate.found.entry)
                    .expect("a test entry");
                (entry, candidate.found.extent.records.first, candidate.score)
            })
            .collect()
    }

    fn near(found: f32, wanted: f32) {
        assert!(
            (found - wanted).abs() <= 1e-6,
            "{found} is not {wanted} within 1e-6"
        );
    }

    fn small() -> Table {
        Table::new()
            .doc("note", 1, 1, "alpha beta")
            .doc("note", 2, 1, "alpha alpha gamma delta")
            .doc("note", 3, 1, "gamma")
    }

    #[test]
    fn a_score_is_bm25ss_lucene_formula() {
        let found = ranked(&small(), "alpha gamma", &[], 10);
        assert_eq!(found.iter().map(|hit| hit.0).collect::<Vec<_>>(), [2, 3, 1]);
        for (hit, wanted) in found.iter().zip([0.409_939_47, 0.278_815_72, 0.226_898_31]) {
            near(hit.2, wanted);
        }
        let near_tie = Table::new()
            .doc("note", 1, 1, "aa bb cc cc cc cc cc")
            .doc("note", 2, 1, "aa aa aa aa aa bb cc")
            .doc("note", 3, 1, "zz zz zz zz zz zz zz");
        let found = ranked(&near_tie, "aa bb cc", &[], 10);
        assert_eq!(found.iter().map(|hit| hit.0).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(found[0].2.to_bits(), 1_062_103_658);
        assert_eq!(found[1].2.to_bits(), 1_062_103_657);
    }

    #[test]
    fn a_repeated_query_word_counts_at_each_place() {
        let found = ranked(&small(), "alpha alpha gamma", &[], 10);
        for (hit, wanted) in found.iter().zip([0.654_551, 0.453_796_63, 0.278_815_72]) {
            near(hit.2, wanted);
        }
        assert_eq!(found.iter().map(|hit| hit.0).collect::<Vec<_>>(), [2, 1, 3]);
    }

    #[test]
    fn a_deleted_document_counts_in_no_statistic() {
        let with_dead = Table::new()
            .doc("note", 1, 1, "alpha beta")
            .doc(
                "note",
                9,
                1,
                "alpha alpha alpha alpha alpha alpha alpha alpha",
            )
            .dead()
            .doc("note", 2, 1, "alpha alpha gamma delta")
            .doc("note", 3, 1, "gamma");
        let found = ranked(&with_dead, "alpha gamma", &[], 10);
        let alone = ranked(&small(), "alpha gamma", &[], 10);
        assert_eq!(found, alone);
    }

    #[test]
    fn equal_scores_come_in_entry_record_and_part_order() {
        let table = Table::new()
            .doc("note", 5, 3, "alpha")
            .doc("note", 2, 9, "alpha")
            .doc("note", 2, 4, "alpha")
            .part(1, 2)
            .doc("note", 2, 4, "alpha")
            .part(0, 2)
            .doc("note", 2, 4, "alpha");
        let found = ranked(&table, "alpha", &[], 10);
        assert_eq!(
            found.iter().map(|hit| (hit.0, hit.1)).collect::<Vec<_>>(),
            [(2, 4), (2, 4), (2, 4), (2, 9), (5, 3)]
        );
        let parts: Vec<_> = Bm25 {
            index: &table,
            k1: 1.2,
            b: 0.75,
            omit: Omit::default(),
            batch: 4096,
        }
        .rank(&query("alpha", &[]), 10)
        .expect("a ranking")
        .candidates
        .iter()
        .take(3)
        .map(|candidate| candidate.found.extent.part)
        .collect();
        assert_eq!(parts, [None, Some((0, 2)), Some((1, 2))]);
    }

    #[test]
    fn a_kind_filter_changes_no_score() {
        let table = Table::new()
            .doc("note", 1, 1, "alpha beta")
            .doc("session", 2, 1, "alpha alpha gamma delta")
            .doc("other", 3, 1, "gamma");
        let all = ranked(&table, "alpha gamma", &[], 10);
        let score_of = |found: &[(u8, u64, f32)], entry: u8| {
            found.iter().find(|hit| hit.0 == entry).map(|hit| hit.2)
        };
        let sessions = ranked(&table, "alpha gamma", &["session"], 10);
        assert_eq!(sessions.len(), 1);
        assert_eq!(score_of(&sessions, 2), score_of(&all, 2));
        let two = ranked(&table, "alpha gamma", &["session", "other"], 10);
        assert_eq!(two.iter().map(|hit| hit.0).collect::<Vec<_>>(), [2, 3]);
        assert_eq!(score_of(&two, 3), score_of(&all, 3));
        assert_eq!(ranked(&table, "alpha", &["nothing"], 10), []);
    }

    #[test]
    fn a_term_in_no_document_adds_nothing_and_a_query_with_no_term_finds_nothing() {
        let table = small();
        assert_eq!(
            ranked(&table, "alpha unheard", &[], 10),
            ranked(&table, "alpha", &[], 10)
        );
        assert_eq!(ranked(&table, "unheard", &[], 10), []);
        assert_eq!(ranked(&table, "the and of", &[], 10), []);
        assert_eq!(ranked(&table, "", &[], 10), []);
    }

    #[test]
    fn a_tie_across_the_limit_is_cut_by_the_tie_order() {
        let mut table = Table::new();
        for entry in (1..=9).rev() {
            table = table.doc("note", entry, 1, "alpha");
        }
        table = table.doc("note", 10, 1, "alpha alpha");
        let found = ranked(&table, "alpha", &[], 4);
        assert_eq!(
            found.iter().map(|hit| hit.0).collect::<Vec<_>>(),
            [10, 1, 2, 3]
        );
    }

    #[test]
    fn a_tie_of_many_documents_is_read_in_batches_and_cut_by_the_tie_order() {
        let mut table = Table::new();
        for first in (1..=10_000).rev() {
            table = table.doc("note", 1, first, "alpha");
        }
        let bm25 = Bm25 {
            index: &table,
            k1: 1.2,
            b: 0.75,
            omit: Omit::default(),
            batch: 1000,
        };
        let found = bm25.rank(&query("alpha", &[]), 3).expect("a ranking");
        assert_eq!(
            found
                .candidates
                .iter()
                .map(|hit| hit.found.extent.records.first)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(table.asked.borrow().iter().max(), Some(&1000));
    }

    #[test]
    fn a_portuguese_querys_function_words_are_never_looked_up() {
        let table = Table::new().doc("note", 1, 1, "status que pelo");
        let says = Says::new(Some(Language::Portuguese));
        let rank = |omit: Omit<'_>| {
            table.looked_up.borrow_mut().clear();
            Bm25 {
                index: &table,
                k1: 1.2,
                b: 0.75,
                omit,
                batch: 4096,
            }
            .rank(&query("que pelo status", &[]), 10)
            .expect("a ranking");
            table.looked_up.borrow().clone()
        };
        let with = rank(Omit {
            words: &[],
            languages: &[Language::Portuguese],
            detect: Some(&says),
        });
        assert_eq!(with, ["status"]);
        let without = rank(Omit::default());
        assert_eq!(without, ["que", "pelo", "status"]);
    }

    #[test]
    fn a_limit_of_nothing_and_an_empty_index_find_nothing() {
        assert_eq!(ranked(&small(), "alpha", &[], 0), []);
        assert_eq!(ranked(&Table::new(), "alpha", &[], 10), []);
        let all_dead = Table::new().doc("note", 1, 1, "alpha").dead();
        assert_eq!(ranked(&all_dead, "alpha", &[], 10), []);
    }

    #[test]
    fn an_index_error_is_returned_not_hidden() {
        let table = small().failing();
        let bm25 = Bm25 {
            index: &table,
            k1: 1.2,
            b: 0.75,
            omit: Omit::default(),
            batch: 4096,
        };
        assert!(matches!(
            bm25.rank(&query("alpha", &[]), 5),
            Err(IndexError::Failed { .. })
        ));
    }

    #[test]
    fn a_posting_outside_the_bound_is_an_index_error() {
        let inconsistent_for = |table: &Table, query: &Query| {
            let bm25 = Bm25 {
                index: table,
                k1: 1.2,
                b: 0.75,
                omit: Omit::default(),
                batch: 4096,
            };
            matches!(bm25.rank(query, 5), Err(IndexError::Inconsistent(_)))
        };
        let inconsistent =
            |table: &Table, kinds: &[&str]| inconsistent_for(table, &query("alpha", kinds));
        for doc in [3, 4_000_000] {
            let table = small().posting_at(doc);
            assert!(inconsistent(&table, &[]), "a posting at {doc}");
            assert!(inconsistent(&table, &["note"]), "a posting at {doc}");
        }
        for doc in [3, 4_000_000] {
            let table = small().holding_at(doc);
            assert!(inconsistent(&table, &["note"]), "a holding at {doc}");
            assert!(
                inconsistent_for(&table, &scoped("alpha", &[], &[1])),
                "a holding of entries at {doc}"
            );
        }
        assert!(!inconsistent(&small().posting_at(2), &["note"]));
        assert!(!inconsistent(&small().holding_at(2), &["note"]));
        assert!(!inconsistent_for(
            &small().holding_at(2),
            &scoped("alpha", &[], &[1])
        ));
    }

    #[test]
    fn a_small_entry_among_a_large_one_still_fills_its_list() {
        let mut table = Table::new();
        for first in 1..=60 {
            table = table.doc("note", 1, first, "alpha alpha alpha");
        }
        for first in 1..=5 {
            table = table.doc("note", 2, first, "alpha beta beta beta beta beta beta");
        }
        let ranked = bm25(&table)
            .rank(&scoped("alpha", &[], &[2]), 5)
            .expect("a ranking");
        assert_eq!(ranked.candidates.len(), 5);
        assert!(ranked.candidates.iter().all(|c| c.found.entry == id(2)));
        assert!(ranked.unscoped.iter().all(|c| c.found.entry == id(1)));
    }

    #[test]
    fn an_entry_filter_changes_no_score() {
        let table = Table::new()
            .doc("note", 1, 1, "alpha beta")
            .doc("note", 2, 1, "alpha alpha gamma delta")
            .doc("note", 3, 1, "alpha gamma gamma gamma")
            .doc("note", 3, 2, "gamma");
        let wide = bm25(&table)
            .rank(&query("alpha gamma", &[]), 10)
            .expect("a ranking")
            .candidates;
        let narrow = bm25(&table)
            .rank(&scoped("alpha gamma", &[], &[2]), 10)
            .expect("a ranking")
            .candidates;
        let bits = |list: &[Candidate]| {
            list.iter()
                .find(|c| c.found.entry == id(2))
                .map(|c| c.score.to_bits())
        };
        assert!(bits(&narrow).is_some());
        assert_eq!(bits(&narrow), bits(&wide));
    }

    #[test]
    fn the_unscoped_list_is_empty_with_no_scope_and_the_best_of_everything_with_one() {
        let table = small();
        let without = bm25(&table)
            .rank(&query("alpha gamma", &[]), 2)
            .expect("a ranking");
        assert!(without.unscoped.is_empty());
        assert_eq!(without.candidates.len(), 2);
        let with = bm25(&table)
            .rank(&scoped("alpha gamma", &[], &[3]), 2)
            .expect("a ranking");
        assert_eq!(with.unscoped, without.candidates);
        assert_eq!(entries_of(&with.candidates), [(3, 1)]);
    }

    #[test]
    fn the_unscoped_list_keeps_the_kinds_asked() {
        let table = Table::new()
            .doc("note", 1, 1, "alpha alpha alpha")
            .doc("session", 2, 1, "alpha beta")
            .doc("session", 3, 1, "alpha beta gamma");
        let ranked = bm25(&table)
            .rank(&scoped("alpha", &["session"], &[3]), 10)
            .expect("a ranking");
        assert_eq!(entries_of(&ranked.unscoped), [(2, 1), (3, 1)]);
        assert_eq!(entries_of(&ranked.candidates), [(3, 1)]);
    }

    #[test]
    fn a_scope_and_a_kind_narrow_together() {
        let table = Table::new()
            .doc("note", 1, 1, "alpha")
            .doc("session", 2, 1, "alpha")
            .doc("session", 3, 1, "alpha")
            .doc("note", 4, 1, "alpha");
        let ranked = bm25(&table)
            .rank(&scoped("alpha", &["session"], &[1, 2]), 10)
            .expect("a ranking");
        assert_eq!(entries_of(&ranked.candidates), [(2, 1)]);
    }

    #[test]
    fn a_scope_of_no_entry_finds_nothing_and_asks_the_index_for_nothing() {
        let table = small();
        let ranked = bm25(&table)
            .rank(&scoped("alpha", &[], &[]), 10)
            .expect("a ranking");
        assert!(ranked.candidates.is_empty() && ranked.unscoped.is_empty());
        let ranked = bm25(&table)
            .rank(&scoped("alpha", &["note"], &[]), 10)
            .expect("a ranking");
        assert!(ranked.candidates.is_empty() && ranked.unscoped.is_empty());
        assert_eq!(table.holdings.get(), 0);
    }

    #[test]
    fn the_entry_filter_is_asked_once_and_never_with_no_scope() {
        let table = small();
        bm25(&table)
            .rank(&query("alpha", &[]), 10)
            .expect("a ranking");
        assert_eq!(table.holdings.get(), 0);
        bm25(&table)
            .rank(&scoped("alpha gamma", &[], &[1, 2, 3]), 10)
            .expect("a ranking");
        assert_eq!(table.holdings.get(), 1);
    }

    #[test]
    fn a_tie_across_the_limit_is_cut_by_the_tie_order_in_both_lists() {
        let mut table = Table::new();
        for entry in (1..=9).rev() {
            table = table.doc("note", entry, 1, "alpha");
        }
        let ranked = bm25(&table)
            .rank(&scoped("alpha", &[], &[3, 5, 7, 9]), 2)
            .expect("a ranking");
        assert_eq!(entries_of(&ranked.candidates), [(3, 1), (5, 1)]);
        assert_eq!(entries_of(&ranked.unscoped), [(1, 1), (2, 1)]);
    }
}
