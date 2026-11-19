pub mod hit;
pub mod keys;
pub mod log;
pub mod pack;
pub mod scope;
pub mod select;

use std::cell::RefCell;
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
use crate::lookup::Lookup;
use crate::position::{self, Manifest, Position, Standing};
use crate::rank::{self, Candidate, Expanded, Query, Ranker};
use hit::{Hit, Readback};
use log::{Ended, QueryLog, Record};
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
        hits,
        apart,
        expanded,
        query: &request.query.text,
    });
    let unlogged = log.append(&line).err().map(|error| error.to_string());
    Finished { result, unlogged }
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
    let mut shorts: Vec<Short> = candidates
        .iter()
        .chain(&apart)
        .map(|candidate| candidate.found.entry.short())
        .collect();
    shorts.sort_unstable();
    shorts.dedup();
    let shared = if shorts.is_empty() {
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
    };
    let selection = RefCell::new(Selection::new(request.limits));
    let mut left_out = 0u64;
    let mut back = Readback::new(entries, readers, tokens, request.chunk);
    let mut gather = |candidates: Vec<Candidate>, apart: bool, budget: usize| {
        let read = candidates
            .into_iter()
            .filter(|candidate| selection.borrow_mut().admits(&candidate.found))
            .filter_map(|candidate| match back.hit(candidate.found) {
                Some(mut hit) => {
                    hit.shared_short = shared.contains(&hit.found.entry.short());
                    hit.apart = apart;
                    Some(Ok::<Hit, Error>(hit))
                }
                None => {
                    left_out += 1;
                    None
                }
            });
        pack::pack(
            read,
            tokens,
            shown,
            &mut |hit| selection.borrow_mut().keeps(hit),
            budget,
        )
    };
    let hits = gather(candidates, false, request.budget)?;
    let apart_hits = if request.apart_budget == 0 {
        Vec::new()
    } else {
        gather(apart, true, request.apart_budget)?
    };
    let skipped = selection.borrow().skipped();
    Ok(Output {
        hits,
        apart: apart_hits,
        searched,
        stand,
        left_out,
        skipped,
        scope: scoped,
        expanded,
    })
}
