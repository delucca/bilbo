use std::collections::HashSet;

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

use crate::index::IndexError;
use crate::lookup::Found;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub kinds: Vec<String>,
    /// Sorted, distinct.
    pub entries: Option<Vec<Id>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub found: Found,
    pub score: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ranked {
    pub candidates: Vec<Candidate>,
    pub unscoped: Vec<Candidate>,
}

pub trait Ranker {
    fn rank(&self, query: &Query, limit: usize) -> Result<Ranked, IndexError>;
}

type Place = (Id, Generation, u64, u64, Option<(u32, u32)>);

fn place(found: &Found) -> Place {
    let extent = &found.extent;
    (
        found.entry,
        extent.generation,
        extent.records.first,
        extent.records.count,
        extent.part,
    )
}

fn merge(lists: Vec<Vec<Candidate>>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    let mut fused = Vec::new();
    let single = lists.len() == 1;
    for list in lists {
        for candidate in list {
            if single || seen.insert(place(&candidate.found)) {
                fused.push(candidate);
            }
        }
    }
    fused
}

pub fn fuse(lists: Vec<Ranked>) -> Ranked {
    let (candidates, unscoped) = lists
        .into_iter()
        .map(|ranked| (ranked.candidates, ranked.unscoped))
        .unzip();
    Ranked {
        candidates: merge(candidates),
        unscoped: merge(unscoped),
    }
}

pub(crate) fn order(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| place(&a.found).cmp(&place(&b.found)))
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::{Generation, RecordRange};

    use crate::index::Extent;
    use crate::lookup::Found;
    use crate::testing::id;

    use super::{Candidate, Ranked, fuse};

    fn candidate(entry: u8, first: u64, score: f32) -> Candidate {
        Candidate {
            found: Found {
                entry: id(entry),
                kind: "note".to_owned(),
                extent: Extent {
                    generation: Generation(1),
                    records: RecordRange { first, count: 1 },
                    part: None,
                    date: 0,
                },
                digest: 0,
            },
            score,
        }
    }

    fn places(list: &[Candidate]) -> Vec<(u8, u64)> {
        list.iter()
            .map(|c| (c.found.entry.to_bytes()[0], c.found.extent.records.first))
            .collect()
    }

    fn ranked(candidates: Vec<Candidate>) -> Ranked {
        Ranked {
            candidates,
            unscoped: Vec::new(),
        }
    }

    #[test]
    fn one_ranked_fuses_to_itself() {
        let list = Ranked {
            candidates: vec![
                candidate(1, 1, 3.0),
                candidate(1, 1, 2.0),
                candidate(2, 1, 1.0),
            ],
            unscoped: vec![candidate(1, 1, 3.0), candidate(1, 1, 3.0)],
        };
        assert_eq!(fuse(vec![list.clone()]), list);
        assert_eq!(fuse(Vec::new()), Ranked::default());
    }

    #[test]
    fn a_document_two_lists_give_is_taken_once_at_its_first_place() {
        let first = vec![candidate(1, 1, 3.0), candidate(2, 1, 2.0)];
        let second = vec![
            candidate(2, 1, 9.0),
            candidate(3, 1, 1.0),
            candidate(1, 1, 8.0),
        ];
        let fused = fuse(vec![ranked(first), ranked(second)]).candidates;
        assert_eq!(places(&fused), [(1, 1), (2, 1), (3, 1)]);
        assert!((fused[1].score - 2.0).abs() < f32::EPSILON);
        let mut other_part = candidate(1, 1, 1.0);
        other_part.found.extent.part = Some((0, 2));
        let fused = fuse(vec![
            ranked(vec![candidate(1, 1, 3.0)]),
            ranked(vec![other_part]),
        ]);
        assert_eq!(fused.candidates.len(), 2);
    }

    #[test]
    fn two_rankers_unscoped_lists_are_fused_as_their_candidates_are() {
        let one = Ranked {
            candidates: Vec::new(),
            unscoped: vec![candidate(1, 1, 3.0), candidate(2, 1, 2.0)],
        };
        let other = Ranked {
            candidates: Vec::new(),
            unscoped: vec![candidate(2, 1, 9.0), candidate(3, 1, 1.0)],
        };
        let fused = fuse(vec![one, other]);
        assert_eq!(places(&fused.unscoped), [(1, 1), (2, 1), (3, 1)]);
        assert!((fused.unscoped[1].score - 2.0).abs() < f32::EPSILON);
        assert!(fused.candidates.is_empty());
    }
}
