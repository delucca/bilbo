pub mod hit;
pub mod keys;
pub mod log;
pub mod pack;
pub mod scope;
pub mod select;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::SystemTime;

use bilbo_entries::reader::Reader;
use bilbo_entries::tokens::Tokens;
use bilbo_store::entries::{Change, Entries, StoreError};
use bilbo_store::id::Short;
use bilbo_store::labels::Labels;

use crate::chunk::Size;
use crate::index::IndexError;
use crate::lookup::{Found, Lookup};
use crate::position::{self, Manifest, Position, Standing};
use crate::rank::{self, Candidate, Expanded, Query, Ranker};
use hit::{Hit, Readback};
use log::{Ended, QueryLog, Record};
use pack::Place;
use scope::Unnamed;
use select::{Limits, Selection, Skipped};

#[derive(Debug, Clone)]
pub struct Request {
    pub query: Query,
    pub manifest: Manifest,
    pub chunk: Size,
    pub candidates: usize,
    pub limits: Limits,
    pub scope: Vec<String>,
    pub budget: usize,
    pub apart_budget: usize,
    pub page: u64,
    pub at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scoped {
    pub kept: u64,
    pub unnamed: Vec<(String, Unnamed)>,
    pub worded: bool,
    pub best: u64,
    pub outside: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stand {
    Current,
    Behind(u64),
    Unknown,
    NoIndex,
    OtherManifest,
    Withheld,
}

#[derive(Clone, Copy)]
pub enum Access<'a> {
    Read {
        index: &'a dyn Lookup,
        rankers: &'a [&'a dyn Ranker],
    },
    NoIndex,
    Withheld,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub hits: Vec<Hit>,
    pub apart: Vec<Hit>,
    pub searched: u64,
    pub stand: Stand,
    pub left_out: u64,
    pub skipped: Skipped,
    pub scope: Option<Scoped>,
    pub expanded: Vec<Expanded>,
    pub more: bool,
    pub ended: Option<u64>,
}

impl Output {
    fn without_hits(stand: Stand) -> Output {
        Output {
            hits: Vec::new(),
            apart: Vec::new(),
            searched: 0,
            stand,
            left_out: 0,
            skipped: Skipped::default(),
            scope: None,
            expanded: Vec::new(),
            more: false,
            ended: None,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Store(StoreError),
    Index(IndexError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Index(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Index(error) => Some(error),
        }
    }
}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Error {
        Error::Store(error)
    }
}

impl From<IndexError> for Error {
    fn from(error: IndexError) -> Error {
        Error::Index(error)
    }
}

#[derive(Debug)]
pub struct Finished {
    pub result: Result<Output, Error>,
    pub unlogged: Option<String>,
}

pub struct Reads<'a> {
    pub entries: &'a dyn Entries,
    pub labels: &'a dyn Labels,
}

pub fn run(
    request: &Request,
    reads: &Reads<'_>,
    readers: &[Box<dyn Reader>],
    tokens: &dyn Tokens,
    access: Access<'_>,
    shown: &dyn Fn(&Hit) -> String,
    log: &mut dyn QueryLog,
) -> Finished {
    let result = search(request, reads, readers, tokens, access, shown);
    let (ended, searched, hits, apart, named, expanded) = match &result {
        Err(_) => (
            Ended::Failed,
            0,
            [].as_slice(),
            [].as_slice(),
            None,
            [].as_slice(),
        ),
        Ok(output) => {
            let ended = match output.stand {
                Stand::Withheld => Ended::Withheld,
                Stand::NoIndex => Ended::NoIndex,
                Stand::OtherManifest => Ended::OtherManifest,
                _ if output.hits.is_empty() && output.apart.is_empty() => Ended::Nothing,
                _ => Ended::Hits,
            };
            (
                ended,
                output.searched,
                output.hits.as_slice(),
                output.apart.as_slice(),
                output.scope.as_ref().map(|scoped| scoped.kept),
                output.expanded.as_slice(),
            )
        }
    };
    let line = log::line(&Record {
        at: request.at,
        ended,
        searched,
        kinds: &request.query.kinds,
        scope: &request.scope,
        named,
        page: request.page,
        hits,
        apart,
        expanded,
        query: &request.query.text,
    });
    let unlogged = log.append(&line).err().map(|error| error.to_string());
    Finished { result, unlogged }
}

pub fn shared_shorts(
    mut shorts: Vec<Short>,
    entries: &dyn Entries,
) -> Result<BTreeSet<Short>, StoreError> {
    shorts.sort_unstable();
    shorts.dedup();
    Ok(if shorts.is_empty() {
        BTreeSet::new()
    } else {
        let mut ending: BTreeMap<Short, usize> = BTreeMap::new();
        for id in entries.by_shorts(&shorts)? {
            *ending.entry(id.short()).or_default() += 1;
        }
        ending
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .map(|(short, _)| short)
            .collect()
    })
}

pub fn stand(
    found: Option<&Position>,
    manifest: &Manifest,
    entries: &dyn Entries,
) -> Result<Stand, StoreError> {
    let Some(found) = found else {
        return Ok(Stand::NoIndex);
    };
    if found.manifest != *manifest {
        return Ok(Stand::OtherManifest);
    }
    let (changes, now) = entries.changes(&found.cursor)?;
    Ok(match position::standing(Some(found), manifest, &now) {
        Standing::At(_) if changes.is_empty() => Stand::Current,
        Standing::At(_) => {
            let changed: BTreeSet<_> = changes.iter().map(Change::id).collect();
            Stand::Behind(changed.len() as u64)
        }
        _ => Stand::Unknown,
    })
}

fn search(
    request: &Request,
    reads: &Reads<'_>,
    readers: &[Box<dyn Reader>],
    tokens: &dyn Tokens,
    access: Access<'_>,
    shown: &dyn Fn(&Hit) -> String,
) -> Result<Output, Error> {
    let Reads { entries, labels } = *reads;
    let (index, rankers) = match access {
        Access::Read { index, rankers } => (index, rankers),
        Access::NoIndex => return Ok(Output::without_hits(Stand::NoIndex)),
        Access::Withheld => return Ok(Output::without_hits(Stand::Withheld)),
    };
    let found = index.position()?;
    let stand = match stand(found.as_ref(), &request.manifest, entries)? {
        Stand::NoIndex => return Ok(Output::without_hits(Stand::NoIndex)),
        Stand::OtherManifest => return Ok(Output::without_hits(Stand::OtherManifest)),
        other => other,
    };
    let scope = if request.scope.is_empty() {
        None
    } else {
        Some(scope::resolve(&request.scope, entries, labels)?)
    };
    if let Some(scope) = scope.as_ref().filter(|scope| scope.entries.is_empty()) {
        let mut output = Output::without_hits(stand);
        output.scope = Some(Scoped {
            kept: 0,
            unnamed: scope.unnamed.clone(),
            worded: scope.worded,
            best: 0,
            outside: BTreeMap::new(),
        });
        return Ok(output);
    }
    let searched = index.totals()?.entries;
    let scoped_query;
    let query = match &scope {
        Some(scope) => {
            scoped_query = Query {
                entries: Some(scope.entries.clone()),
                ..request.query.clone()
            };
            &scoped_query
        }
        None => &request.query,
    };
    let mut lists = Vec::with_capacity(rankers.len());
    for ranker in rankers {
        lists.push(ranker.rank(query, request.candidates)?);
    }
    let fused = rank::fuse(lists);
    let expanded = fused.expanded;
    let scoped = scope.map(|scope| {
        let mut outside: BTreeMap<String, u64> = BTreeMap::new();
        for candidate in &fused.unscoped {
            if scope.entries.binary_search(&candidate.found.entry).is_err() {
                *outside.entry(candidate.found.kind.clone()).or_default() += 1;
            }
        }
        Scoped {
            kept: scope.entries.len() as u64,
            unnamed: scope.unnamed,
            worded: scope.worded,
            best: fused.unscoped.len() as u64,
            outside,
        }
    });
    let candidates = fused.candidates;
    let apart = fused.apart;
    let shorts: Vec<Short> = candidates
        .iter()
        .chain(&apart)
        .map(|candidate| candidate.found.entry.short())
        .collect();
    let shared = shared_shorts(shorts, entries)?;
    let selection = RefCell::new(Selection::new(request.limits));
    let room = if apart.is_empty() {
        request.budget + request.apart_budget
    } else {
        request.budget
    };
    let context = Context {
        back: RefCell::new(Readback::new(entries, readers, tokens, request.chunk)),
        left_out: Cell::new(0),
        shared,
        tokens,
        shown,
    };
    let mut main = List {
        place: Place::default(),
        rest: candidates.into_iter(),
        apart: false,
    };
    let mut notes_list = List {
        place: if request.apart_budget == 0 {
            Place::closed()
        } else {
            Place::default()
        },
        rest: apart.into_iter(),
        apart: true,
    };
    let walked = walk(request.page, || {
        let hits = round(&context, &mut main, room, &selection)?;
        let apart_hits = round(&context, &mut notes_list, request.apart_budget, &selection)?;
        Ok::<Round, Error>(Round {
            hits,
            apart: apart_hits,
            more: !(main.over() && notes_list.over()),
        })
    })?;
    let skipped = selection.borrow().skipped();
    Ok(Output {
        hits: walked.hits,
        apart: walked.apart,
        searched,
        stand,
        left_out: context.left_out.get(),
        skipped,
        scope: scoped,
        expanded,
        more: walked.more,
        ended: walked.ended,
    })
}

#[derive(Default)]
struct Round {
    hits: Vec<Hit>,
    apart: Vec<Hit>,
    more: bool,
}

struct Walked {
    hits: Vec<Hit>,
    apart: Vec<Hit>,
    more: bool,
    ended: Option<u64>,
}

fn walk<E>(page: u64, mut round: impl FnMut() -> Result<Round, E>) -> Result<Walked, E> {
    let mut last_shown = None;
    let mut number = 0;
    loop {
        number += 1;
        let current = round()?;
        let shown = !current.hits.is_empty() || !current.apart.is_empty();
        if number == page {
            return Ok(if shown {
                Walked {
                    hits: current.hits,
                    apart: current.apart,
                    more: current.more,
                    ended: None,
                }
            } else {
                Walked {
                    hits: Vec::new(),
                    apart: Vec::new(),
                    more: false,
                    ended: last_shown,
                }
            });
        }
        if shown {
            last_shown = Some(number);
        }
        if !current.more {
            return Ok(Walked {
                hits: Vec::new(),
                apart: Vec::new(),
                more: false,
                ended: last_shown,
            });
        }
    }
}

struct Context<'a> {
    back: RefCell<Readback<'a>>,
    left_out: Cell<u64>,
    shared: BTreeSet<Short>,
    tokens: &'a dyn Tokens,
    shown: &'a dyn Fn(&Hit) -> String,
}

struct List {
    place: Place,
    rest: std::vec::IntoIter<Candidate>,
    apart: bool,
}

impl List {
    fn over(&self) -> bool {
        self.place.ended() || (self.rest.len() == 0 && !self.place.holds())
    }
}

fn round(
    context: &Context<'_>,
    list: &mut List,
    budget: usize,
    selection: &RefCell<Selection>,
) -> Result<Vec<Hit>, Error> {
    let apart = list.apart;
    let admits = |found: &Found| {
        if apart {
            selection.borrow_mut().admits_apart(found)
        } else {
            selection.borrow_mut().admits(found)
        }
    };
    if let Some(waited) = list.place.take_held()
        && admits(&waited.found)
    {
        list.place.hold(waited);
    }
    let read = list
        .rest
        .by_ref()
        .filter(|candidate| admits(&candidate.found))
        .filter_map(
            |candidate| match context.back.borrow_mut().hit(candidate.found) {
                Some(mut hit) => {
                    hit.shared_short = context.shared.contains(&hit.found.entry.short());
                    hit.apart = apart;
                    Some(Ok::<Hit, Error>(hit))
                }
                None => {
                    context.left_out.set(context.left_out.get() + 1);
                    None
                }
            },
        );
    list.place.pack(
        read,
        context.tokens,
        context.shown,
        &mut |hit| selection.borrow_mut().keeps(hit),
        budget,
    )
}

#[cfg(test)]
mod tests {
    use bilbo_store::entries::{Generation, RecordRange};
    use bilbo_store::id::Id;

    use super::{Hit, Round, walk};
    use crate::index::Extent;
    use crate::lookup::Found;

    fn hit() -> Hit {
        Hit {
            found: Found {
                entry: Id::from_bytes([1; 16]),
                kind: "note".to_owned(),
                extent: Extent {
                    generation: Generation(1),
                    records: RecordRange { first: 1, count: 1 },
                    part: None,
                    date: 0,
                },
                digest: 0,
            },
            preface: None,
            text: "text".to_owned(),
            cut: false,
            apart: false,
            headed: false,
            shared_short: false,
        }
    }

    fn rounds_for(page: u64) -> (usize, Option<u64>, bool) {
        let mut made = 0;
        let walked = walk(page, || {
            made += 1;
            Ok::<Round, ()>(Round {
                hits: if made == 1 { vec![hit()] } else { Vec::new() },
                apart: Vec::new(),
                more: false,
            })
        })
        .expect("a walk");
        assert!(walked.hits.is_empty() && walked.apart.is_empty());
        (made, walked.ended, walked.more)
    }

    #[test]
    fn a_page_far_past_the_end_walks_no_further_than_the_lists() {
        let (second, ended, more) = rounds_for(2);
        assert_eq!((ended, more), (Some(1), false));
        assert_eq!(rounds_for(1_000_000_000), (second, Some(1), false));
        assert_eq!(second, 1);
    }
}
