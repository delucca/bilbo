use crate::index::IndexError;
use crate::lookup::{Edits, Filter, Found, Lookup, Near, Posting, Totals};
use crate::position::Position;

pub struct Alone<'a> {
    inner: &'a dyn Lookup,
    kinds: Vec<String>,
    docs: Vec<u32>,
    totals: Totals,
}

impl<'a> Alone<'a> {
    pub fn new(inner: &'a dyn Lookup, kinds: &[String]) -> Result<Alone<'a>, IndexError> {
        let mut docs = Vec::new();
        let mut length = 0u64;
        let mut asked: Vec<&String> = Vec::new();
        for kind in kinds {
            if asked.contains(&kind) {
                continue;
            }
            asked.push(kind);
            docs.extend(inner.holding(&Filter::Kind(kind))?);
            length = length.saturating_add(inner.length(kind)?);
        }
        docs.sort_unstable();
        let bound = inner.totals()?.bound;
        Ok(Alone {
            inner,
            kinds: asked.into_iter().cloned().collect(),
            totals: Totals {
                documents: docs.len() as u64,
                entries: 0,
                length,
                bound,
            },
            docs,
        })
    }

    fn mine(&self, postings: Vec<Posting>) -> Vec<Posting> {
        postings
            .into_iter()
            .filter(|posting| self.docs.binary_search(&posting.doc).is_ok())
            .collect()
    }
}

impl Lookup for Alone<'_> {
    fn position(&self) -> Result<Option<Position>, IndexError> {
        self.inner.position()
    }

    fn totals(&self) -> Result<Totals, IndexError> {
        Ok(self.totals)
    }

    fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
        Ok(self.mine(self.inner.postings(term)?))
    }

    fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
        let mut near = Vec::new();
        for each in self.inner.near(term, edits)? {
            let documents = self.postings(&each.term)?.len() as u64;
            if documents > 0 {
                near.push(Near {
                    term: each.term,
                    documents,
                });
            }
        }
        Ok(near)
    }

    fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
        Ok(self.mine(self.inner.pairs(first, second)?))
    }

    fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
        Ok(self
            .inner
            .holding(filter)?
            .into_iter()
            .filter(|doc| self.docs.binary_search(doc).is_ok())
            .collect())
    }

    fn length(&self, kind: &str) -> Result<u64, IndexError> {
        if self.kinds.iter().any(|held| held == kind) {
            self.inner.length(kind)
        } else {
            Ok(0)
        }
    }

    fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
        self.inner.found(docs)
    }
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::Cursor;

    use crate::chunk::Size;
    use crate::index::IndexError;
    use crate::lookup::{Edits, Filter, Found, Lookup, Near, Posting, Totals};
    use crate::position::{Manifest, Position};
    use crate::rank::{Query, Ranked, Ranker};
    use crate::recall::keys::defaults;
    use crate::testing::{Says, Table, id};

    use super::Alone;

    fn kinds(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn placed() -> Position {
        Position {
            manifest: Manifest::new(
                Size {
                    tokens: 64,
                    overlap: 8,
                },
                "bytes/1",
                "lines/1",
            ),
            cursor: Cursor::at([4; 16], 3),
            failed: Vec::new(),
        }
    }

    struct Placed {
        table: Table,
        damaged: Option<&'static str>,
    }

    impl Lookup for Placed {
        fn position(&self) -> Result<Option<Position>, IndexError> {
            Ok(Some(placed()))
        }

        fn totals(&self) -> Result<Totals, IndexError> {
            self.table.totals()
        }

        fn postings(&self, term: &str) -> Result<Vec<Posting>, IndexError> {
            self.table.postings(term)
        }

        fn near(&self, term: &str, edits: Edits) -> Result<Vec<Near>, IndexError> {
            self.table.near(term, edits)
        }

        fn pairs(&self, first: &str, second: &str) -> Result<Vec<Posting>, IndexError> {
            self.table.pairs(first, second)
        }

        fn holding(&self, filter: &Filter<'_>) -> Result<Vec<u32>, IndexError> {
            if matches!(filter, Filter::Kind(kind) if Some(*kind) == self.damaged) {
                return Err(IndexError::Inconsistent("a kind that is damaged"));
            }
            self.table.holding(filter)
        }

        fn length(&self, kind: &str) -> Result<u64, IndexError> {
            self.table.length(kind)
        }

        fn found(&self, docs: &[u32]) -> Result<Vec<Found>, IndexError> {
            self.table.found(docs)
        }
    }

    fn shared() -> Table {
        Table::new()
            .doc("lines", 1, 1, "garden shed shed")
            .doc("memo", 2, 1, "garden hose")
            .doc("lines", 3, 1, "shed hose garden")
    }

    #[test]
    fn a_view_holds_the_documents_of_its_kinds_and_no_other() {
        let table = shared();
        let view = Alone::new(&table, &kinds(&["lines"])).expect("a view");
        let totals = view.totals().expect("totals");
        assert_eq!(
            (
                totals.documents,
                totals.length,
                totals.bound,
                totals.entries
            ),
            (2, 6, 3, 0)
        );
        let docs = |postings: Vec<crate::lookup::Posting>| -> Vec<u32> {
            postings.into_iter().map(|posting| posting.doc).collect()
        };
        assert_eq!(docs(view.postings("garden").expect("postings")), [0, 2]);
        assert_eq!(docs(view.postings("hose").expect("postings")), [2]);
        assert_eq!(
            docs(view.pairs("garden", "hose").expect("pairs")),
            Vec::<u32>::new()
        );
        assert_eq!(docs(view.pairs("shed", "hose").expect("pairs")), [2]);
        assert_eq!(
            view.holding(&Filter::Kind("lines")).expect("holding"),
            [0, 2]
        );
        assert!(
            view.holding(&Filter::Kind("memo"))
                .expect("holding")
                .is_empty()
        );
        assert_eq!(
            view.holding(&Filter::Entries(&[id(1), id(2)]))
                .expect("holding"),
            [0]
        );
        assert_eq!(view.length("lines").expect("length"), 6);
        assert_eq!(view.length("memo").expect("length"), 0);
        let found = view.found(&[0, 1]).expect("found");
        assert_eq!(found, table.found(&[0, 1]).expect("found"));
        let inner = Placed {
            table: shared(),
            damaged: None,
        };
        let view = Alone::new(&inner, &kinds(&["lines"])).expect("a view");
        assert_eq!(view.position().expect("position"), Some(placed()));
        let inner = Placed {
            table: shared(),
            damaged: Some("memo"),
        };
        let view = Alone::new(&inner, &kinds(&["lines"])).expect("a view");
        let error = view
            .holding(&Filter::Kind("memo"))
            .expect_err("the inner index's own error");
        assert!(matches!(
            error,
            IndexError::Inconsistent("a kind that is damaged")
        ));
    }

    #[test]
    fn a_near_term_counts_the_views_documents_and_one_only_others_hold_is_none() {
        let table = Table::new()
            .doc("lines", 1, 1, "carrot")
            .doc("memo", 2, 1, "carrot carat")
            .doc("memo", 3, 1, "carrot");
        let whole = table.near("carot", Edits::One).expect("near");
        assert!(whole.contains(&Near {
            term: "carrot".to_owned(),
            documents: 3
        }));
        let view = Alone::new(&table, &kinds(&["lines"])).expect("a view");
        assert_eq!(
            view.near("carot", Edits::One).expect("near"),
            [Near {
                term: "carrot".to_owned(),
                documents: 1
            }]
        );
    }

    #[test]
    fn two_kinds_are_one_view_and_a_kind_with_no_document_is_empty() {
        let table = shared().doc("other", 4, 1, "garden");
        let view = Alone::new(&table, &kinds(&["lines", "memo"])).expect("a view");
        let totals = view.totals().expect("totals");
        assert_eq!((totals.documents, totals.length), (3, 8));
        let docs: Vec<u32> = view
            .postings("garden")
            .expect("postings")
            .into_iter()
            .map(|posting| posting.doc)
            .collect();
        assert_eq!(docs, [0, 1, 2]);

        let empty = Alone::new(&table, &kinds(&["absent"])).expect("a view");
        let totals = empty.totals().expect("totals");
        assert_eq!((totals.documents, totals.length), (0, 0));
        let says = Says::new(None);
        let section = defaults();
        let query = Query {
            text: "garden".to_owned(),
            kinds: Vec::new(),
            apart: Vec::new(),
            entries: None,
        };
        let ranked = section
            .bm25(&empty, &says)
            .rank(&query, 10)
            .expect("ranked");
        assert_eq!(ranked, Ranked::default());
    }
}
