use std::collections::{HashMap, HashSet};

use bilbo_store::entries::Generation;
use bilbo_store::id::Id;

use crate::index::IndexError;
use crate::lookup::Found;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub found: Found,
    pub score: f32,
}

pub trait Ranker {
    fn rank(&self, query: &Query, limit: usize) -> Result<Vec<Candidate>, IndexError>;
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

pub fn fuse(lists: Vec<Vec<Candidate>>) -> Vec<Candidate> {
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

pub fn cap(ranked: Vec<Candidate>, per_entry: usize) -> Vec<Candidate> {
    if per_entry == 0 {
        return ranked;
    }
    let mut taken: HashMap<Id, usize> = HashMap::new();
    ranked
        .into_iter()
        .filter(|candidate| {
            let count = taken.entry(candidate.found.entry).or_default();
            *count += 1;
            *count <= per_entry
        })
        .collect()
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

    use super::{Candidate, cap, fuse};

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

    #[test]
    fn one_list_fuses_to_itself() {
        let list = vec![
            candidate(1, 1, 3.0),
            candidate(1, 1, 2.0),
            candidate(2, 1, 1.0),
        ];
        assert_eq!(fuse(vec![list.clone()]), list);
        assert_eq!(fuse(Vec::new()), []);
    }

    #[test]
    fn a_document_two_lists_give_is_taken_once_at_its_first_place() {
        let first = vec![candidate(1, 1, 3.0), candidate(2, 1, 2.0)];
        let second = vec![
            candidate(2, 1, 9.0),
            candidate(3, 1, 1.0),
            candidate(1, 1, 8.0),
        ];
        let fused = fuse(vec![first, second]);
        assert_eq!(places(&fused), [(1, 1), (2, 1), (3, 1)]);
        assert!((fused[1].score - 2.0).abs() < f32::EPSILON);
        let mut other_part = candidate(1, 1, 1.0);
        other_part.found.extent.part = Some((0, 2));
        let fused = fuse(vec![vec![candidate(1, 1, 3.0)], vec![other_part]]);
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn no_cap_keeps_the_list() {
        let list = vec![
            candidate(1, 1, 3.0),
            candidate(1, 2, 2.0),
            candidate(1, 3, 1.0),
        ];
        assert_eq!(cap(list.clone(), 0), list);
    }

    #[test]
    fn a_cap_keeps_each_entrys_first_hits_in_rank_order() {
        let list = vec![
            candidate(1, 1, 5.0),
            candidate(2, 1, 4.0),
            candidate(1, 2, 3.0),
            candidate(1, 3, 2.0),
            candidate(2, 2, 1.0),
        ];
        assert_eq!(places(&cap(list.clone(), 1)), [(1, 1), (2, 1)]);
        assert_eq!(places(&cap(list, 2)), [(1, 1), (2, 1), (1, 2), (2, 2)]);
    }
}
