use std::cell::RefCell;
use std::collections::HashMap;

use crate::index::IndexError;
use crate::lookup::{Edits, Filter, Found, Lookup, Near, Posting, Totals};
use crate::position::Position;

#[derive(Default)]
struct Kept {
    failed: bool,
    reading: Option<String>,
    totals: Option<Totals>,
    documents: HashMap<String, Vec<u32>>,
    lengths: HashMap<String, u64>,
    near: HashMap<(String, Edits), Vec<Near>>,
    postings: HashMap<String, Vec<Posting>>,
    pairs: HashMap<(String, String), Vec<Posting>>,
    share: Option<Vec<u32>>,
}

pub struct Held<'a> {
    inner: &'a dyn Lookup,
    kinds: Vec<String>,
    kept: RefCell<Kept>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Reads,
    Kept,
}

pub struct Face<'h, 'a> {
    held: &'h Held<'a>,
    side: Side,
}

fn documents(inner: &dyn Lookup, kept: &mut Kept, kind: &str) -> Result<Vec<u32>, IndexError> {
    if let Some(found) = kept.documents.get(kind) {
        return Ok(found.clone());
    }
    let found = inner.holding(&Filter::Kind(kind))?;
    kept.documents.insert(kind.to_owned(), found.clone());
    Ok(found)
}

fn narrowed(
    inner: &dyn Lookup,
    kinds: &[String],
    kept: &mut Kept,
    postings: &[Posting],
) -> Result<Vec<Posting>, IndexError> {
    if kept.share.is_none() {
        let mut share = Vec::new();
        for kind in kinds {
            if kept.reading.as_deref() != Some(kind.as_str()) {
                share.extend(documents(inner, kept, kind)?);
            }
        }
        share.sort_unstable();
        share.dedup();
        kept.share = Some(share);
    }
    let share = kept.share.as_deref().unwrap_or_default();
    Ok(postings
        .iter()
        .filter(|posting| share.binary_search(&posting.doc).is_ok())
        .copied()
        .collect())
}

impl<'a> Held<'a> {
    pub fn new(inner: &'a dyn Lookup, kinds: &[String]) -> Result<Held<'a>, IndexError> {
        let held = Held {
            inner,
            kinds: kinds.to_vec(),
            kept: RefCell::new(Kept::default()),
        };
        let mut largest = 0;
        let mut reading: Option<&String> = None;
        for kind in kinds {
            let size = held
                .asking(|inner, _, kept| documents(inner, kept, kind).map(|found| found.len()))?;
            if reading.is_none() || size > largest {
                largest = size;
                reading = Some(kind);
            }
        }
        held.kept.borrow_mut().reading = reading.cloned();
        Ok(held)
    }

    pub fn reading(&self) -> Option<String> {
        self.kept.borrow().reading.clone()
    }

    pub fn face(&self, kind: &str) -> Face<'_, 'a> {
        let side = if self.kept.borrow().reading.as_deref() == Some(kind) {
            Side::Reads
        } else {
            Side::Kept
        };
        Face { held: self, side }
    }

    fn asking<T>(
        &self,
        ask: impl FnOnce(&dyn Lookup, &[String], &mut Kept) -> Result<T, IndexError>,
    ) -> Result<T, IndexError> {
        let mut kept = self.kept.borrow_mut();
        if kept.failed {
            return Err(IndexError::Inconsistent(
                "the index failed earlier in this run",
            ));
        }
        let answered = ask(self.inner, &self.kinds, &mut kept);
        if answered.is_err() {
            kept.failed = true;
        }
        answered
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.asking(|inner, _, kept| {
            if let Some(totals) = kept.totals {
                return Ok(totals);
            }
            let totals = inner.totals()?;
            kept.totals = Some(totals);
            Ok(totals)
        })
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.asking(|inner, _, kept| match filter {
            Filter::Kind(kind) => documents(inner, kept, kind),
            Filter::Entries(_) => inner.holding(filter),
        })
    }

    fn length(&self, kind: &str) -> Result<u64, IndexError> {
        self.asking(|inner, _, kept| {
            if let Some(length) = kept.lengths.get(kind) {
                return Ok(*length);
            }
            let length = inner.length(kind)?;
            kept.lengths.insert(kind.to_owned(), length);
            Ok(length)
        })
    }

    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
        self.asking(|inner, _, kept| {
            let key = (term.to_owned(), edits);
            if let Some(near) = kept.near.get(&key) {
                return Ok(near.clone());
            }
            let near = inner.near(term, edits)?;
            kept.near.insert(key, near.clone());
            Ok(near)
        })
    }

    fn postings(&self, term: &str, side: Side) -> Result<Vec<Posting>, IndexError> {
        self.asking(|inner, kinds, kept| {
            if side == Side::Kept
                && let Some(share) = kept.postings.get(term)
            {
                return Ok(share.clone());
            }
            let whole = inner.postings(term)?;
            if !kept.postings.contains_key(term) {
                let share = narrowed(inner, kinds, kept, &whole)?;
                kept.postings.insert(term.to_owned(), share);
            }
            if side == Side::Reads {
                return Ok(whole);
            }
            Ok(kept.postings.get(term).cloned().unwrap_or_default())
        })
    }

    fn pairs(&self, first: &str, second: &str, side: Side) -> Result<Vec<Posting>, IndexError> {
        self.asking(|inner, kinds, kept| {
            let key = (first.to_owned(), second.to_owned());
            if side == Side::Kept
                && let Some(share) = kept.pairs.get(&key)
            {
                return Ok(share.clone());
            }
            let whole = inner.pairs(first, second)?;
            if !kept.pairs.contains_key(&key) {
                let share = narrowed(inner, kinds, kept, &whole)?;
                kept.pairs.insert(key.clone(), share);
            }
            if side == Side::Reads {
                return Ok(whole);
            }
            Ok(kept.pairs.get(&key).cloned().unwrap_or_default())
        })
    }

    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.asking(|inner, _, _| inner.position())
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.asking(|inner, _, _| inner.found(docs))
    }
}
impl Lookup for Face<'_, '_> {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.held.position()
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        self.held.totals()
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        self.held.postings(term, self.side)
    }

    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
        self.held.near(term, edits)
    }

    fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
        self.held.pairs(first, second, self.side)
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        self.held.holding(filter)
    }

    fn length(&self, kind: &str) -> Result<u64, IndexError> {
        self.held.length(kind)
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.held.found(docs)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

    use super::Held;
    use crate::index::IndexError;
    use crate::lookup::{Edits, Filter, Found, Lookup, Near, Posting, Totals};
    use crate::position::Position;
    use crate::testing::{Table, id};

    struct Counting {
        inner: Table,
        calls: RefCell<Vec<String>>,
        fail_on: Cell<Option<&'static str>>,
    }

    impl Counting {
        fn asked(&self, what: &str) -> usize {
            self.calls
                .borrow()
                .iter()
                .filter(|call| *call == what)
                .count()
        }

        fn seen(&self, what: &str) -> Result<(), IndexError> {
            self.calls.borrow_mut().push(what.to_owned());
            if self.fail_on.get() == Some(what) {
                return Err(IndexError::Failed {
                    action: "read",
                    path: PathBuf::from("/c/index"),
                    error: "boom".to_owned(),
                });
            }
            Ok(())
        }
    }

    impl Lookup for Counting {
        fn position(&self) -> Result<Option<Position>, IndexError> {
            self.seen("position")?;
            self.inner.position()
        }

        fn totals(&self) -> Result<Totals, IndexError> {
            self.seen("totals")?;
            self.inner.totals()
        }

        fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
            self.seen(&format!("postings:{term}"))?;
            self.inner.postings(term)
        }

        fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
            self.seen(&format!("near:{term}:{edits:?}"))?;
            self.inner.near(term, edits)
        }

        fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
            self.seen(&format!("pairs:{first}:{second}"))?;
            self.inner.pairs(first, second)
        }

        fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
            match filter {
                Filter::Kind(kind) => self.seen(&format!("holding:{kind}"))?,
                Filter::Entries(_) => self.seen("holding:entries")?,
            }
            self.inner.holding(filter)
        }

        fn length(&self, kind: &str) -> Result<u64, IndexError> {
            self.seen(&format!("length:{kind}"))?;
            self.inner.length(kind)
        }

        fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
            self.seen("found")?;
            self.inner.found(docs)
        }
    }

    fn counting() -> Counting {
        Counting {
            inner: Table::new()
                .doc("lines", 1, 1, "garden shed shed")
                .doc("memo", 2, 1, "garden hose garden hose")
                .doc("jots", 3, 1, "shed hose garden")
                .doc("lines", 4, 1, "garden hose shed"),
            calls: RefCell::new(Vec::new()),
            fail_on: Cell::new(None),
        }
    }

    fn kinds() -> Vec<String> {
        ["lines", "memo", "jots"].map(str::to_owned).to_vec()
    }

    fn docs(postings: Vec<Posting>) -> Vec<u32> {
        postings.into_iter().map(|posting| posting.doc).collect()
    }

    #[test]
    fn the_second_face_answers_from_what_the_first_set_aside() {
        let table = counting();
        let held = Held::new(&table, &kinds()).expect("a held index");
        let first = held.face("lines");
        let second = held.face("memo");

        let whole = first.postings("garden").expect("postings");
        assert_eq!(docs(whole), [0, 1, 2, 3]);
        assert_eq!(table.asked("postings:garden"), 1);
        first.postings("garden").expect("postings");
        assert_eq!(table.asked("postings:garden"), 2, "two calls, two readings");
        assert_eq!(
            docs(second.postings("garden").expect("postings")),
            [1, 2],
            "the other lists' documents alone"
        );
        assert_eq!(table.asked("postings:garden"), 2);

        assert_eq!(docs(second.postings("hose").expect("postings")), [1, 2]);
        second.postings("hose").expect("postings");
        assert_eq!(table.asked("postings:hose"), 1);

        first.postings("shed").expect("postings");
        assert_eq!(docs(second.postings("shed").expect("postings")), [2]);
        assert_eq!(table.asked("postings:shed"), 1);

        assert_eq!(docs(second.pairs("shed", "hose").expect("pairs")), [2]);
        second.pairs("shed", "hose").expect("pairs");
        assert_eq!(table.asked("pairs:shed:hose"), 1);
        second.pairs("hose", "shed").expect("pairs");
        assert_eq!(
            table.asked("pairs:hose:shed"),
            1,
            "the other way is another question"
        );
        assert_eq!(docs(first.pairs("garden", "hose").expect("pairs")), [1, 3]);
        assert_eq!(docs(first.pairs("garden", "hose").expect("pairs")), [1, 3]);
        assert_eq!(table.asked("pairs:garden:hose"), 2, "two calls, two reads");
        assert_eq!(
            docs(second.pairs("garden", "hose").expect("pairs")),
            [1],
            "the set-aside share"
        );
        assert_eq!(table.asked("pairs:garden:hose"), 2);

        for _ in 0..2 {
            first.near("garde", Edits::One).expect("near");
            second.near("garde", Edits::One).expect("near");
        }
        assert_eq!(table.asked("near:garde:One"), 1);
        let terms = |near: Vec<crate::lookup::Near>| -> Vec<String> {
            near.into_iter().map(|each| each.term).collect()
        };
        assert_eq!(
            terms(first.near("garde", Edits::One).expect("near")),
            ["garden"]
        );
        for _ in 0..2 {
            assert_eq!(
                terms(first.near("hosex", Edits::One).expect("near")),
                ["hose"]
            );
            assert_eq!(
                terms(second.near("hosex", Edits::One).expect("near")),
                ["hose"]
            );
        }
        assert_eq!(table.asked("near:hosex:One"), 1, "a word of its own");
        assert_eq!(table.asked("near:garde:One"), 1);
        second.near("garde", Edits::Two).expect("near");
        assert_eq!(
            table.asked("near:garde:Two"),
            1,
            "one edit and two are two questions"
        );

        for _ in 0..2 {
            first.totals().expect("totals");
            second.totals().expect("totals");
            first.holding(&Filter::Kind("lines")).expect("holding");
            second.holding(&Filter::Kind("lines")).expect("holding");
            first.length("lines").expect("length");
            second.length("lines").expect("length");
        }
        assert_eq!(table.asked("totals"), 1);
        assert_eq!(table.asked("holding:lines"), 1);
        assert_eq!(table.asked("length:lines"), 1);
        for _ in 0..2 {
            assert_eq!(first.length("memo").expect("length"), 4);
            assert_eq!(second.length("jots").expect("length"), 3);
            assert_eq!(second.length("lines").expect("length"), 6);
            assert_eq!(first.length("jots").expect("length"), 3);
        }
        assert_eq!(table.asked("length:memo"), 1);
        assert_eq!(table.asked("length:jots"), 1);
        assert_eq!(table.asked("length:lines"), 1);
        assert_eq!(table.asked("holding:memo"), 1, "once, however it was asked");
        assert_eq!(table.asked("holding:jots"), 1);

        let whole_table = counting();
        let both = Held::new(&whole_table, &kinds()).expect("a held index");
        let (reads, behind) = (both.face("lines"), both.face("memo"));
        let pair = whole_table.inner.pairs("garden", "hose").expect("pairs");
        let memo = pair.iter().find(|posting| posting.doc == 1).expect("memo");
        assert_eq!(
            (memo.freq, memo.length),
            (2, 4),
            "the fixture's own property"
        );
        let word = whole_table.inner.postings("garden").expect("postings");
        let set_aside = |all: &[Posting]| -> Vec<Posting> {
            all.iter()
                .filter(|posting| ![0, 3].contains(&posting.doc))
                .copied()
                .collect()
        };
        assert_eq!(reads.pairs("garden", "hose").expect("pairs"), pair);
        assert_eq!(
            behind.pairs("garden", "hose").expect("pairs"),
            set_aside(&pair)
        );
        assert_eq!(reads.postings("garden").expect("postings"), word);
        assert_eq!(
            behind.postings("garden").expect("postings"),
            set_aside(&word)
        );

        for _ in 0..2 {
            first.position().expect("position");
            second.position().expect("position");
            first.found(&[0]).expect("found");
            second.found(&[0]).expect("found");
            first.holding(&Filter::Entries(&[id(1)])).expect("holding");
            second.holding(&Filter::Entries(&[id(1)])).expect("holding");
        }
        assert_eq!(table.asked("position"), 4);
        assert_eq!(table.asked("found"), 4);
        assert_eq!(table.asked("holding:entries"), 4);

        {
            let table = counting();
            table.fail_on.set(Some("postings:zzz"));
            let held = Held::new(&table, &kinds()).expect("a held index");
            let first = held.face("lines");
            let second = held.face("memo");
            let error = first.postings("zzz").expect_err("the table's error");
            assert_eq!(
                error.to_string(),
                IndexError::Failed {
                    action: "read",
                    path: PathBuf::from("/c/index"),
                    error: "boom".to_owned(),
                }
                .to_string()
            );
            let before = table.calls.borrow().len();
            assert!(second.postings("zzz").is_err());
            assert!(second.totals().is_err());
            assert!(first.found(&[0]).is_err());
            assert_eq!(
                table.calls.borrow().len(),
                before,
                "nothing was asked again"
            );
        }
    }
}
