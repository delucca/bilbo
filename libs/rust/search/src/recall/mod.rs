pub mod hit;
pub mod keys;
pub mod log;
pub mod pack;

use std::collections::BTreeSet;
use std::fmt;
use std::time::SystemTime;

use bilbo_kinds::reader::Reader;
use bilbo_kinds::tokens::Tokens;
use bilbo_store::entries::{Change, Entries, StoreError};

use crate::chunk::Size;
use crate::index::IndexError;
use crate::lookup::Lookup;
use crate::position::{self, Manifest, Standing};
use crate::rank::{self, Query, Ranker};
use hit::{Hit, Readback};
use log::{Ended, QueryLog, Record};

#[derive(Debug, Clone)]
pub struct Request {
    pub query: Query,
    pub manifest: Manifest,
    pub chunk: Size,
    pub candidates: usize,
    pub cap: usize,
    pub budget: usize,
    pub at: SystemTime,
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
    pub searched: u64,
    pub stand: Stand,
    pub left_out: u64,
}

impl Output {
    fn without_hits(stand: Stand) -> Output {
        Output {
            hits: Vec::new(),
            searched: 0,
            stand,
            left_out: 0,
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

pub fn run(
    request: &Request,
    entries: &dyn Entries,
    readers: &[Box<dyn Reader>],
    tokens: &dyn Tokens,
    access: Access<'_>,
    shown: &dyn Fn(&Hit) -> String,
    log: &mut dyn QueryLog,
) -> Finished {
    let result = search(request, entries, readers, tokens, access, shown);
    let (ended, searched, hits) = match &result {
        Err(_) => (Ended::Failed, 0, [].as_slice()),
        Ok(output) => {
            let ended = match output.stand {
                Stand::Withheld => Ended::Withheld,
                Stand::NoIndex => Ended::NoIndex,
                Stand::OtherManifest => Ended::OtherManifest,
                _ if output.hits.is_empty() => Ended::Nothing,
                _ => Ended::Hits,
            };
            (ended, output.searched, output.hits.as_slice())
        }
    };
    let line = log::line(&Record {
        at: request.at,
        ended,
        searched,
        kinds: &request.query.kinds,
        hits,
        query: &request.query.text,
    });
    let unlogged = log.append(&line).err().map(|error| error.to_string());
    Finished { result, unlogged }
}

fn search(
    request: &Request,
    entries: &dyn Entries,
    readers: &[Box<dyn Reader>],
    tokens: &dyn Tokens,
    access: Access<'_>,
    shown: &dyn Fn(&Hit) -> String,
) -> Result<Output, Error> {
    let (index, rankers) = match access {
        Access::Read { index, rankers } => (index, rankers),
        Access::NoIndex => return Ok(Output::without_hits(Stand::NoIndex)),
        Access::Withheld => return Ok(Output::without_hits(Stand::Withheld)),
    };
    let Some(found) = index.position()? else {
        return Ok(Output::without_hits(Stand::NoIndex));
    };
    if found.manifest != request.manifest {
        return Ok(Output::without_hits(Stand::OtherManifest));
    }
    let (changes, now) = entries.changes(&found.cursor)?;
    let stand = match position::standing(Some(&found), &request.manifest, &now) {
        Standing::At(_) if changes.is_empty() => Stand::Current,
        Standing::At(_) => {
            let changed: BTreeSet<_> = changes.iter().map(Change::id).collect();
            Stand::Behind(changed.len() as u64)
        }
        _ => Stand::Unknown,
    };
    let searched = index.totals()?.entries;
    let mut lists = Vec::with_capacity(rankers.len());
    for ranker in rankers {
        lists.push(ranker.rank(&request.query, request.candidates)?);
    }
    let candidates = rank::cap(rank::fuse(lists), request.cap);
    let mut left_out = 0u64;
    let mut back = Readback::new(entries, readers, tokens, request.chunk);
    let read = candidates
        .into_iter()
        .filter_map(|candidate| match back.hit(candidate.found) {
            Some(hit) => Some(Ok::<Hit, Error>(hit)),
            None => {
                left_out += 1;
                None
            }
        });
    let hits = pack::pack(read, tokens, shown, request.budget)?;
    Ok(Output {
        hits,
        searched,
        stand,
        left_out,
    })
}
