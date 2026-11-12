use std::collections::{BTreeMap, BTreeSet};

use bilbo_kinds::reader::Reader;
use bilbo_store::entries::{Change, Cursor, Entries};
use bilbo_store::id::Id;

use crate::fill::Fill;
use crate::index::Index;
use crate::position::{self, MAX_FAILED, Manifest, Position, Standing};
use crate::reindex;
use crate::tokenizer::Tokenizer;
use crate::work::{Error, Request};

/// The payload holds a count in one digit.
const MOST_TRIES: u8 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Current,
    Work(Work),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Never committed, or its payload is not a position.
    Unmade,
    Manifest,
    /// Its cursor is of another catalog, or ahead of this one's newest change.
    Catalog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work(Box<Parts>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Parts {
    kind: Kind,
    manifest: Manifest,
    ids: BTreeSet<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Rebuild(Reason),
    Grow {
        found: Position,
        changes: Vec<Change>,
        now: Cursor,
    },
}

impl Work {
    /// `None` when the index is continued.
    pub fn rebuilds(&self) -> Option<Reason> {
        match &self.0.kind {
            Kind::Rebuild(why) => Some(*why),
            Kind::Grow { .. } => None,
        }
    }

    /// How many distinct entries the work names: every entry of the store for a rebuild.
    pub fn entries(&self) -> usize {
        self.0.ids.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Entries whose newest generation was put in the index, also when it gave no chunk.
    pub entries: u64,
    pub chunks: u64,
    /// By kind.
    pub unread: BTreeMap<String, u64>,
    /// The entries this run attempted and left out, with why.
    pub failed: Vec<(Id, String)>,
    pub index_bytes: u64,
    pub rebuilt: bool,
}

/// Writes nothing.
/// A failed entry that was tried fewer than `tries` times is tried again; 0 retries nothing.
pub fn plan(
    manifest: &Manifest,
    tries: u8,
    entries: &dyn Entries,
    index: &mut dyn Index,
) -> Result<Plan, Error> {
    let found = index.position()?;
    let since = found
        .as_ref()
        .map_or_else(Cursor::start, |found| found.cursor.clone());
    let (changes, now) = entries.changes(&since)?;
    let standing = position::standing(found.as_ref(), manifest, &now);
    let (Standing::At(_), Some(found)) = (standing, found.as_ref()) else {
        let reason = match standing {
            Standing::Unmade | Standing::At(_) => Reason::Unmade,
            Standing::OtherManifest => Reason::Manifest,
            Standing::OtherCatalog => Reason::Catalog,
        };
        let everything = if found.is_some() {
            entries.changes(&Cursor::start())?.0
        } else {
            changes
        };
        return Ok(Plan::Work(Work(Box::new(Parts {
            kind: Kind::Rebuild(reason),
            manifest: manifest.clone(),
            ids: everything.iter().map(Change::id).collect(),
        }))));
    };
    let retry = found.failed.iter().any(|(_, tried)| *tried < tries);
    if changes.is_empty() && !retry {
        return Ok(Plan::Current);
    }
    let mut ids: BTreeSet<Id> = changes.iter().map(Change::id).collect();
    if retry {
        ids.extend(
            found
                .failed
                .iter()
                .filter(|(_, tried)| *tried < tries)
                .map(|(id, _)| *id),
        );
    }
    Ok(Plan::Work(Work(Box::new(Parts {
        kind: Kind::Grow {
            found: found.clone(),
            changes,
            now,
        },
        manifest: manifest.clone(),
        ids,
    }))))
}

/// Does the work, and commits once, or not at all: a return with an error leaves the index at its
/// last commit and holding no writer.
pub fn apply(
    work: Work,
    request: &Request,
    entries: &dyn Entries,
    readers: &[Box<dyn Reader>],
    tokenizer: &Tokenizer,
    index: &mut dyn Index,
) -> Result<Output, Error> {
    apply_within(
        work, request, entries, readers, tokenizer, index, MAX_FAILED,
    )
}

fn apply_within(
    work: Work,
    request: &Request,
    entries: &dyn Entries,
    readers: &[Box<dyn Reader>],
    tokenizer: &Tokenizer,
    index: &mut dyn Index,
    max_failed: usize,
) -> Result<Output, Error> {
    if Manifest::new(request.chunk, tokenizer.id, &request.render) != work.0.manifest {
        return Err(Error::Manifest);
    }
    let Parts {
        kind,
        manifest,
        ids,
    } = *work.0;
    let done = match kind {
        Kind::Rebuild(_) => reindex::rebuild(
            manifest, request, entries, readers, tokenizer, index, max_failed,
        )
        .map(|made| Output {
            entries: made.entries,
            chunks: made.chunks,
            unread: made.unread,
            failed: made.failed,
            index_bytes: made.index_bytes,
            rebuilt: true,
        }),
        Kind::Grow {
            found,
            changes,
            now,
        } => grow(
            (found, changes, now, ids, manifest),
            request,
            entries,
            readers,
            tokenizer,
            index,
            max_failed,
        ),
    };
    if done.is_err() {
        index.rollback();
    }
    done
}

type Found = (Position, Vec<Change>, Cursor, BTreeSet<Id>, Manifest);

fn grow(
    (found, changes, now, ids, manifest): Found,
    request: &Request,
    entries: &dyn Entries,
    readers: &[Box<dyn Reader>],
    tokenizer: &Tokenizer,
    index: &mut dyn Index,
    max_failed: usize,
) -> Result<Output, Error> {
    let mut position = Position {
        manifest,
        cursor: now,
        failed: Vec::new(),
    };
    if position.encode().is_none() {
        return Err(Error::Manifest);
    }
    let meta_changed: BTreeSet<Id> = changes
        .iter()
        .filter_map(|change| match change {
            Change::Meta(id) => Some(*id),
            Change::Records { .. } => None,
        })
        .collect();
    let fill = Fill {
        size: request.chunk,
        workers: request.workers,
        entries,
        readers,
        tokens: tokenizer.tokens.as_ref(),
        grown: Some(&meta_changed),
    };
    let made = fill.run(&ids, index)?;
    let mut listed: BTreeMap<Id, u8> = found.failed.iter().copied().collect();
    for id in &ids {
        listed.remove(id);
    }
    for (id, _) in &made.failed {
        let tries = found
            .failed
            .iter()
            .find(|(listed, _)| listed == id)
            .map_or(1, |(_, tries)| (tries + 1).min(MOST_TRIES));
        listed.insert(*id, tries);
    }
    if listed.len() > max_failed {
        return Err(Error::Failed(listed.len()));
    }
    position.failed = listed.into_iter().collect();
    let bytes = index.commit(&position)?.bytes;
    Ok(Output {
        entries: made.entries,
        chunks: made.chunks,
        unread: made.unread,
        failed: made.failed,
        index_bytes: bytes,
        rebuilt: false,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bilbo_kinds::reader::Reader;
    use bilbo_store::entries::{Change, Cursor};

    use super::{Kind, Parts, Work, apply_within};
    use crate::chunk::Size;
    use crate::position::{Manifest, Position};
    use crate::testing::{Bytes, Commits, Four, OnePassage, id};
    use crate::tokenizer::Tokenizer;
    use crate::work::{Error, Request};

    #[test]
    fn more_listed_entries_than_a_position_holds_stops_a_growth_and_commits_nothing() {
        let request = Request {
            chunk: Size {
                tokens: 12,
                overlap: 2,
            },
            render: "lines/1".to_owned(),
            workers: 1,
        };
        let readers: Vec<Box<dyn Reader>> = vec![Box::new(OnePassage)];
        let tokenizer = Tokenizer {
            tokens: Box::new(Bytes),
            id: "bytes/1",
        };
        let manifest = Manifest::new(request.chunk, tokenizer.id, &request.render);
        let now = Cursor::at([1; 16], 4);
        let found = Position {
            manifest: manifest.clone(),
            cursor: now.clone(),
            failed: vec![(id(1), 1), (id(3), 1)],
        };
        let work = Work(Box::new(Parts {
            kind: Kind::Grow {
                found,
                changes: vec![Change::Meta(id(4))],
                now,
            },
            manifest,
            ids: BTreeSet::from([id(4)]),
        }));
        let mut index = Commits::default();
        let error = apply_within(
            work.clone(),
            &request,
            &Four,
            &readers,
            &tokenizer,
            &mut index,
            2,
        )
        .expect_err("a failure");
        assert!(matches!(error, Error::Failed(3)), "{error:?}");
        assert_eq!((index.rollbacks, index.commits.len()), (1, 0));
        apply_within(work, &request, &Four, &readers, &tokenizer, &mut index, 3)
            .expect("a growth at the bound");
        assert_eq!(index.commits[0].failed.len(), 3);
    }
}
