use std::collections::{HashMap, HashSet};

use bilbo_store::id::Id;

use super::hit::Hit;
use crate::analyze;
use crate::lookup::Found;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub per_entry: usize,
    pub per_record: usize,
    pub copy_share: f64,
    pub copy_words: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    pub copies: u64,
    pub parts: u64,
    pub capped: u64,
}

pub(super) struct Selection {
    limits: Limits,
    kept: HashMap<Id, Vec<(u64, u64)>>,
    digests: HashSet<u64>,
    runs: HashSet<String>,
    skipped: Skipped,
}

fn ends(found: &Found) -> (u64, u64) {
    let records = found.extent.records;
    (
        records.first,
        records
            .first
            .saturating_add(records.count.saturating_sub(1)),
    )
}

/// The most kept ranges that hold one record of `first..=last`.
fn deepest(kept: &[(u64, u64)], first: u64, last: u64) -> usize {
    let mut events: Vec<(u128, i8)> = Vec::new();
    for &(from, to) in kept {
        if to < first || from > last {
            continue;
        }
        events.push((u128::from(from.max(first)), 1));
        events.push((u128::from(to.min(last)) + 1, -1));
    }
    events.sort_unstable();
    let (mut depth, mut deepest) = (0i64, 0i64);
    for (_, step) in events {
        depth += i64::from(step);
        deepest = deepest.max(depth);
    }
    usize::try_from(deepest).unwrap_or(0)
}

fn runs_of(text: &str, length: usize) -> HashSet<String> {
    let words = analyze::every_word(text);
    if words.is_empty() {
        return HashSet::new();
    }
    if words.len() <= length {
        return HashSet::from([words.join(" ")]);
    }
    words.windows(length).map(|run| run.join(" ")).collect()
}

impl Selection {
    pub(super) fn new(limits: Limits) -> Selection {
        Selection {
            limits,
            kept: HashMap::new(),
            digests: HashSet::new(),
            runs: HashSet::new(),
            skipped: Skipped::default(),
        }
    }

    pub(super) fn admits(&mut self, found: &Found) -> bool {
        let kept = self.kept.get(&found.entry).map_or(&[][..], Vec::as_slice);
        if self.limits.per_entry != 0 && kept.len() >= self.limits.per_entry {
            self.skipped.capped += 1;
            return false;
        }
        if self.limits.per_record != 0 {
            let (first, last) = ends(found);
            if deepest(kept, first, last) >= self.limits.per_record {
                self.skipped.parts += 1;
                return false;
            }
        }
        if self.limits.copy_share > 0.0 && self.digests.contains(&found.digest) {
            self.skipped.copies += 1;
            return false;
        }
        true
    }

    pub(super) fn keeps(&mut self, hit: &Hit) -> bool {
        let mut runs = HashSet::new();
        if self.limits.copy_share > 0.0 {
            runs = runs_of(&hit.text, self.limits.copy_words);
            if !runs.is_empty() {
                let shared = runs.iter().filter(|run| self.runs.contains(*run)).count();
                if shared as f64 / runs.len() as f64 >= self.limits.copy_share {
                    self.skipped.copies += 1;
                    return false;
                }
            }
        }
        self.runs.extend(runs);
        self.digests.insert(hit.found.digest);
        self.kept
            .entry(hit.found.entry)
            .or_default()
            .push(ends(&hit.found));
        true
    }

    pub(super) fn skipped(&self) -> Skipped {
        self.skipped
    }
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::{Generation, RecordRange};

    use super::{Limits, Selection, Skipped};
    use crate::index::Extent;
    use crate::lookup::Found;
    use crate::recall::hit::Hit;
    use crate::testing::id;

    const LIMITS: Limits = Limits {
        per_entry: 0,
        per_record: 2,
        copy_share: 0.5,
        copy_words: 5,
    };

    fn found(entry: u8, first: u64, count: u64, part: u32, digest: u64) -> Found {
        Found {
            entry: id(entry),
            kind: "note".to_owned(),
            extent: Extent {
                generation: Generation(1),
                records: RecordRange { first, count },
                part: (part > 0).then(|| (part - 1, 9)),
                date: 0,
            },
            digest,
        }
    }

    fn hit(entry: u8, first: u64, count: u64, part: u32, digest: u64, text: &str) -> Hit {
        Hit {
            found: found(entry, first, count, part, digest),
            preface: None,
            text: text.to_owned(),
            cut: false,
            shared_short: false,
        }
    }

    fn plain(entry: u8, digest: u64, text: &str) -> Hit {
        hit(entry, 1, 1, 0, digest, text)
    }

    fn keep(selection: &mut Selection, hit: &Hit) -> bool {
        selection.admits(&hit.found) && selection.keeps(hit)
    }

    const TEN: &str = "a b c d e f g h i j";

    #[test]
    fn a_hit_with_exactly_half_its_runs_above_is_a_copy() {
        let mut selection = Selection::new(LIMITS);
        assert!(keep(&mut selection, &plain(1, 1, TEN)));
        assert!(!keep(&mut selection, &plain(2, 2, "a b c d e f g x y z")));
        assert!(keep(&mut selection, &plain(3, 3, "a b c d e f x y z w")));
        assert_eq!(selection.skipped().copies, 1);
        let mut strict = Selection::new(Limits {
            copy_share: 0.7,
            ..LIMITS
        });
        assert!(keep(&mut strict, &plain(1, 1, TEN)));
        assert!(keep(&mut strict, &plain(2, 2, "a b c d e f g x y z")));
    }

    #[test]
    fn a_hit_passed_over_adds_nothing_for_later_hits() {
        let mut selection = Selection::new(LIMITS);
        assert!(keep(&mut selection, &plain(1, 1, TEN)));
        assert!(!keep(
            &mut selection,
            &plain(2, 2, "a b c d e f g h i j x1 x2 x3 x4 x5 x6")
        ));
        assert!(keep(
            &mut selection,
            &plain(3, 3, "g h i j x1 x2 x3 x4 x5 x6")
        ));
    }

    #[test]
    fn the_preface_is_no_part_of_a_hits_runs() {
        let mut selection = Selection::new(LIMITS);
        let preface = "one two three four five six seven eight nine ten eleven twelve";
        for (n, text) in [(1, "a b c d e"), (2, "f g h i j")] {
            let mut hit = plain(n, u64::from(n), text);
            hit.preface = Some(preface.to_owned());
            assert!(keep(&mut selection, &hit));
        }
    }

    #[test]
    fn a_text_under_the_run_length_is_one_run_and_a_text_of_no_word_has_none() {
        let mut selection = Selection::new(LIMITS);
        assert!(keep(&mut selection, &plain(1, 1, "alpha beta gamma")));
        assert!(!keep(&mut selection, &plain(2, 2, "alpha  beta\ngamma")));
        assert!(keep(
            &mut selection,
            &plain(3, 3, "q alpha beta gamma delta eps")
        ));
        assert!(keep(&mut selection, &plain(4, 4, "... ---")));
        assert!(keep(&mut selection, &plain(5, 5, "")));
    }

    #[test]
    fn an_equal_chunk_is_passed_over_by_its_digest() {
        let mut selection = Selection::new(LIMITS);
        let first = plain(1, 7, "alpha");
        assert!(selection.admits(&first.found));
        assert_eq!(selection.skipped().copies, 0);
        assert!(selection.keeps(&first));
        assert!(!selection.admits(&plain(2, 7, "other").found));
        assert_eq!(selection.skipped().copies, 1);
    }

    #[test]
    fn two_parts_of_one_record_are_kept_and_the_third_passed_over() {
        let mut selection = Selection::new(LIMITS);
        for part in 1..=2 {
            let hit = hit(1, 7, 1, part, u64::from(part), &format!("w{part} x y z q"));
            assert!(keep(&mut selection, &hit));
        }
        assert!(!keep(&mut selection, &hit(1, 7, 1, 3, 3, "w3 x y z q")));
        assert_eq!(selection.skipped().parts, 1);
    }

    #[test]
    fn a_third_window_that_shares_a_record_with_two_kept_ones_is_passed_over() {
        let kept = |ranges: &[(u64, u64)]| {
            let mut selection = Selection::new(LIMITS);
            for (n, (first, count)) in ranges.iter().enumerate() {
                let text = format!("u{n}a u{n}b u{n}c u{n}d u{n}e");
                assert!(keep(
                    &mut selection,
                    &hit(1, *first, *count, 0, n as u64, &text)
                ));
            }
            selection
        };
        let mut selection = kept(&[(1, 2), (2, 1)]);
        assert!(!selection.admits(&found(1, 2, 2, 0, 90)));
        assert!(selection.admits(&found(1, 3, 2, 0, 91)));
        assert!(selection.admits(&found(2, 2, 2, 0, 92)));
        let mut selection = kept(&[(1, 2), (2, 2)]);
        assert!(!selection.admits(&found(1, 2, 1, 0, 93)));
        assert!(selection.admits(&found(1, 1, 1, 0, 94)));
        assert!(selection.admits(&found(1, 3, 1, 0, 95)));
    }

    #[test]
    fn parts_of_another_record_of_the_entry_are_kept() {
        let mut selection = Selection::new(LIMITS);
        for part in 1..=2 {
            let text = format!("p{part}a p{part}b p{part}c p{part}d p{part}e");
            assert!(keep(
                &mut selection,
                &hit(1, 7, 1, part, u64::from(part), &text)
            ));
        }
        assert!(selection.admits(&found(1, 9, 1, 1, 10)));
    }

    #[test]
    fn the_record_cap_never_walks_a_ranges_records() {
        let big = 1u64 << 62;
        let mut selection = Selection::new(LIMITS);
        for n in 0..2u64 {
            let text = format!("v{n}a v{n}b v{n}c v{n}d v{n}e");
            assert!(keep(&mut selection, &hit(1, 1, big, 0, n, &text)));
        }
        assert!(!selection.admits(&found(1, 1, big, 0, 5)));
        assert!(selection.admits(&found(1, big + 1, big, 0, 6)));
    }

    #[test]
    fn the_caps_count_kept_hits_only() {
        let mut selection = Selection::new(LIMITS);
        let same = "r s t u v w";
        assert!(keep(&mut selection, &hit(1, 7, 1, 1, 1, same)));
        assert!(!keep(&mut selection, &hit(1, 7, 1, 2, 2, same)));
        assert!(keep(&mut selection, &hit(1, 7, 1, 3, 3, "k l m n o p")));
        assert!(!keep(&mut selection, &hit(1, 7, 1, 4, 4, "z y x w v u")));
        assert_eq!(
            selection.skipped(),
            Skipped {
                copies: 1,
                parts: 1,
                capped: 0
            }
        );
        let mut selection = Selection::new(Limits {
            per_entry: 1,
            ..LIMITS
        });
        assert!(keep(&mut selection, &plain(2, 1, same)));
        assert!(!keep(&mut selection, &plain(1, 2, same)));
        assert!(keep(&mut selection, &hit(1, 2, 1, 0, 3, "k l m n o p")));
    }

    #[test]
    fn the_entry_cap_bounds_the_kept_hits_of_one_entry_and_zero_sets_none() {
        let mut selection = Selection::new(Limits {
            per_entry: 2,
            ..LIMITS
        });
        for n in 1..=2u64 {
            let text = format!("e{n}a e{n}b e{n}c e{n}d e{n}e");
            assert!(keep(&mut selection, &hit(1, n, 1, 0, n, &text)));
        }
        assert!(!selection.admits(&found(1, 3, 1, 0, 9)));
        assert!(selection.admits(&found(2, 3, 1, 0, 9)));
        assert_eq!(selection.skipped().capped, 1);
        let mut none = Selection::new(LIMITS);
        for n in 1..=5u64 {
            let text = format!("n{n}a n{n}b n{n}c n{n}d n{n}e");
            assert!(keep(&mut none, &hit(1, n, 1, 0, n, &text)));
        }
    }

    #[test]
    fn a_copy_share_of_zero_passes_over_no_copy_and_no_equal_chunk() {
        let mut selection = Selection::new(Limits {
            copy_share: 0.0,
            ..LIMITS
        });
        assert!(keep(&mut selection, &plain(1, 1, TEN)));
        assert!(keep(&mut selection, &plain(2, 1, TEN)));
        assert_eq!(selection.skipped(), Skipped::default());
    }

    #[test]
    fn what_is_passed_over_is_counted_under_its_first_reason() {
        let mut selection = Selection::new(Limits {
            per_entry: 1,
            ..LIMITS
        });
        assert!(keep(&mut selection, &plain(1, 1, TEN)));
        assert!(!selection.admits(&found(1, 2, 1, 0, 1)));
        assert_eq!(
            selection.skipped(),
            Skipped {
                copies: 0,
                parts: 0,
                capped: 1
            }
        );
    }
}
