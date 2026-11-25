pub mod alone;
pub mod held;
pub mod keys;
pub mod log;
pub mod sessions;

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::time::{Duration, SystemTime};

use bilbo_entries::day;
use bilbo_entries::read::{kept_from, outline};
use bilbo_entries::reader::Reader;
use bilbo_store::entries::{Entries, Generation, RecordRange};
use bilbo_store::id::{Id, Short};
use bilbo_store::labels::{CaptureLabel, Labels};

use crate::analyze::Detect;
use crate::index::{instant, millis};
use crate::lookup::{Found, Lookup};
use crate::position::Manifest;
use crate::rank::{Candidate, Query, Ranker};
use crate::recall::{self, Error, Stand};
use alone::Alone;
use held::Held;
use sessions::{Claimed, Sessions};

pub type Line<'a> = &'a dyn Fn(&[u8]) -> Option<String>;

#[derive(Clone)]
pub struct List<'a> {
    pub kind: String,
    pub most: usize,
    pub floor: f64,
    pub line: Option<Line<'a>>,
    pub old_after: Option<Duration>,
    pub lists_belonging: bool,
    pub lists_parts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    thread: String,
    text: String,
}

impl Prompt {
    pub fn new(thread: &str, text: &str) -> Option<Prompt> {
        (!thread.is_empty() && !text.is_empty()).then(|| Prompt {
            thread: thread.to_owned(),
            text: text.to_owned(),
        })
    }

    pub fn thread(&self) -> &str {
        &self.thread
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct First {
    thread: String,
    text: String,
}

impl First {
    pub fn thread(&self) -> &str {
        &self.thread
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug)]
pub enum Turn {
    First(First),
    Later,
    Unmarked(io::Error),
}

pub fn turn(prompt: Prompt, sessions: &mut dyn Sessions) -> Turn {
    match sessions.claim(&prompt.thread) {
        Ok(Claimed::Marked) => Turn::First(First {
            thread: prompt.thread,
            text: prompt.text,
        }),
        Ok(Claimed::Seen) => Turn::Later,
        Err(error) => Turn::Unmarked(error),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub entry: Id,
    pub generation: Generation,
    pub records: RecordRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub from: Vec<Source>,
    pub text: String,
}

#[derive(Clone, Copy)]
pub struct Way<'a> {
    pub label: &'a dyn Fn(Id) -> CaptureLabel,
    pub written: &'a dyn Fn(&[u8]) -> Option<Written>,
}

pub struct Request<'a> {
    pub first: &'a First,
    pub lists: &'a [List<'a>],
    pub manifest: &'a Manifest,
    pub digest: &'a keys::Section,
    pub ranker: &'a recall::keys::Section,
    pub readers: &'a [Box<dyn Reader>],
    pub width: usize,
    pub labels: &'a dyn Labels,
    pub at: SystemTime,
    pub way: Way<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    Entry {
        title: Option<String>,
    },
    Passage {
        records: RecordRange,
        day: String,
        folder: Option<String>,
        line: Option<String>,
        summary: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub entry: Id,
    pub capture: Option<CaptureLabel>,
    pub score: f32,
    pub shared_short: bool,
    pub said: Said,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeldBack {
    pub entry: Id,
    pub capture: Option<CaptureLabel>,
    pub score: f32,
    pub records: Option<RecordRange>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    pub kind: String,
    pub floor: f64,
    pub rows: Vec<Row>,
    pub held_back: Vec<HeldBack>,
    pub gone: u64,
    pub own: u64,
    pub belongs: u64,
    pub old: u64,
    pub part: u64,
    pub shown: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub stand: Stand,
    pub lists: Vec<Chosen>,
}

struct Pick {
    found: Found,
    capture: Option<CaptureLabel>,
    score: f32,
    newest: Generation,
    count: u64,
}

#[derive(Default)]
struct Counts {
    gone: u64,
    own: u64,
    belongs: u64,
    old: u64,
    part: u64,
    shown: u64,
}

struct Taken {
    picks: Vec<Pick>,
    held_back: Vec<HeldBack>,
    counts: Counts,
}

pub fn list(
    request: &Request<'_>,
    index: &dyn Lookup,
    entries: &dyn Entries,
    detect: &dyn Detect,
) -> Result<Listed, Error> {
    let found = index.position()?;
    let stand = match recall::stand(found.as_ref(), request.manifest, entries)? {
        stand @ (Stand::NoIndex | Stand::OtherManifest) => {
            return Ok(Listed {
                stand,
                lists: Vec::new(),
            });
        }
        stand => stand,
    };
    let query = Query {
        text: request
            .first
            .text()
            .chars()
            .take(request.digest.query.value)
            .collect(),
        kinds: Vec::new(),
        apart: Vec::new(),
        entries: None,
    };
    let on: Vec<&List<'_>> = request.lists.iter().filter(|list| list.most > 0).collect();
    let kinds: Vec<String> = on.iter().map(|list| list.kind.clone()).collect();
    let held = Held::new(index, &kinds)?;
    let reading = held
        .reading()
        .and_then(|kind| on.iter().position(|list| list.kind == kind))
        .unwrap_or(0);
    let mut ranked: Vec<Option<Vec<Candidate>>> = on.iter().map(|_| None).collect();
    let order: Vec<usize> = if on.is_empty() {
        Vec::new()
    } else {
        std::iter::once(reading)
            .chain((0..on.len()).filter(|at| *at != reading))
            .collect()
    };
    for at in order {
        let list = on[at];
        let kind = std::slice::from_ref(&list.kind);
        let face = held.face(&list.kind);
        let view = Alone::new(&face, kind)?;
        let candidates = request
            .ranker
            .bm25(&view, detect)
            .rank(&query, request.digest.candidates.value)?
            .candidates;
        ranked[at] = Some(candidates);
    }
    let mut ranked_lists = Vec::new();
    for (list, candidates) in on.into_iter().zip(ranked) {
        let reader = request
            .readers
            .iter()
            .find(|reader| reader.kind() == list.kind)
            .map(AsRef::as_ref);
        ranked_lists.push((list, reader, candidates.unwrap_or_default()));
    }
    let own = if ranked_lists
        .iter()
        .all(|(_, _, candidates)| candidates.is_empty())
    {
        HashSet::new()
    } else {
        own_entries(request, entries)?
    };
    let now = millis(request.at);
    let mut taken = Vec::new();
    let mut shown: HashSet<Id> = HashSet::new();
    for (list, reader, candidates) in ranked_lists {
        let chosen = choose((list, reader), candidates, entries, (&own, &shown), now);
        shown.extend(chosen.picks.iter().map(|pick| pick.found.entry));
        taken.push((list, reader, chosen));
    }
    let shorts: Vec<Short> = taken
        .iter()
        .flat_map(|(_, _, taken)| &taken.picks)
        .map(|pick| pick.found.entry.short())
        .collect();
    let shared = recall::shared_shorts(shorts, entries)?;
    let mut lists = Vec::new();
    for (list, reader, taken) in taken {
        let mut chosen = Chosen {
            kind: list.kind.clone(),
            floor: list.floor,
            rows: Vec::new(),
            held_back: taken.held_back,
            gone: taken.counts.gone,
            own: taken.counts.own,
            belongs: taken.counts.belongs,
            old: taken.counts.old,
            part: taken.counts.part,
            shown: taken.counts.shown,
        };
        if let Some(reader) = reader {
            for pick in taken.picks {
                match row(request, reader, list, entries, &pick) {
                    Some(said) => chosen.rows.push(Row {
                        entry: pick.found.entry,
                        capture: pick.capture,
                        score: pick.score,
                        shared_short: shared.contains(&pick.found.entry.short()),
                        said,
                    }),
                    None => chosen.gone += 1,
                }
            }
        }
        lists.push(chosen);
    }
    Ok(Listed { stand, lists })
}

enum Cause {
    Gone,
    Own,
    Belongs,
    Old,
    Shown,
}

enum Judged {
    Open {
        capture: Option<CaptureLabel>,
        newest: Generation,
        count: u64,
    },
    Done,
}

fn own_entries(request: &Request<'_>, entries: &dyn Entries) -> Result<HashSet<Id>, Error> {
    let kept: Vec<Id> = kept_from(request.first.thread(), entries)?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    if kept.is_empty() {
        return Ok(HashSet::new());
    }
    let mut own: HashSet<Id> = kept.iter().copied().collect();
    own.extend(request.labels.belonging_to(&kept)?);
    Ok(own)
}

fn judge(
    entries: &dyn Entries,
    list: &List<'_>,
    found: &Found,
    (own, shown): (&HashSet<Id>, &HashSet<Id>),
    began_before: Option<i64>,
) -> Result<Judged, Cause> {
    let entry = found.entry;
    let Some(meta) = entries.meta(entry).ok().flatten() else {
        return Err(Cause::Gone);
    };
    let (newest, count) = match entries.extent(entry, None).ok().flatten() {
        Some((newest, count)) if count > 0 => (newest, count),
        _ => return Err(Cause::Gone),
    };
    if own.contains(&entry) {
        return Err(Cause::Own);
    }
    if !list.lists_belonging && meta.labels.id.is_some() {
        return Err(Cause::Belongs);
    }
    if began_before.is_some_and(|limit| found.extent.date < limit) {
        return Err(Cause::Old);
    }
    if meta.labels.id.is_some_and(|above| shown.contains(&above)) {
        return Err(Cause::Shown);
    }
    Ok(Judged::Open {
        capture: meta.labels.capture,
        newest,
        count,
    })
}

fn holds(found: &Found, newest: Generation, count: u64) -> bool {
    let records = found.extent.records;
    found.extent.generation == newest
        && records.count > 0
        && records.first >= 1
        && records
            .first
            .checked_add(records.count)
            .is_some_and(|end| end - 1 <= count)
}

fn choose(
    (list, reader): (&List<'_>, Option<&dyn Reader>),
    candidates: Vec<Candidate>,
    entries: &dyn Entries,
    hidden: (&HashSet<Id>, &HashSet<Id>),
    now: i64,
) -> Taken {
    let grows = reader.is_some_and(Reader::appends_only);
    let began_before = list.old_after.map(|age| {
        let span = i64::try_from(age.as_millis()).unwrap_or(i64::MAX);
        now.saturating_sub(span)
    });
    let mut picks: Vec<Pick> = Vec::new();
    let mut counts = Counts::default();
    let mut judged: HashMap<Id, Judged> = HashMap::new();
    for candidate in candidates {
        if picks.len() == list.most {
            break;
        }
        let found = candidate.found;
        let entry = found.entry;
        if let Entry::Vacant(room) = judged.entry(entry) {
            let verdict = if reader.is_some() {
                judge(entries, list, &found, hidden, began_before)
            } else {
                Err(Cause::Gone)
            };
            room.insert(match verdict {
                Ok(open) => open,
                Err(cause) => {
                    match cause {
                        Cause::Gone => counts.gone += 1,
                        Cause::Own => counts.own += 1,
                        Cause::Belongs => counts.belongs += 1,
                        Cause::Old => counts.old += 1,
                        Cause::Shown => counts.shown += 1,
                    }
                    Judged::Done
                }
            });
        }
        let Some(Judged::Open { newest, count, .. }) = judged.get(&entry) else {
            continue;
        };
        let (newest, count) = (*newest, *count);
        if grows && !holds(&found, newest, count) {
            counts.gone += 1;
            continue;
        }
        if !list.lists_parts && found.extent.part.is_some() {
            counts.part += 1;
            continue;
        }
        let Some(Judged::Open { capture, .. }) = judged.insert(entry, Judged::Done) else {
            continue;
        };
        picks.push(Pick {
            capture,
            newest,
            count,
            score: candidate.score,
            found,
        });
    }
    let (picks, held): (Vec<Pick>, Vec<Pick>) = picks
        .into_iter()
        .partition(|pick| f64::from(pick.score) >= list.floor);
    Taken {
        picks,
        held_back: held
            .into_iter()
            .map(|pick| HeldBack {
                entry: pick.found.entry,
                capture: pick.capture,
                score: pick.score,
                records: grows.then_some(pick.found.extent.records),
            })
            .collect(),
        counts,
    }
}

fn row(
    request: &Request<'_>,
    reader: &dyn Reader,
    list: &List<'_>,
    entries: &dyn Entries,
    pick: &Pick,
) -> Option<Said> {
    let records = entries
        .read(
            pick.found.entry,
            pick.newest,
            RecordRange {
                first: 1,
                count: pick.count,
            },
        )
        .ok()?;
    let records: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
    if !reader.appends_only() {
        return Some(Said::Entry {
            title: reader
                .title(&records)
                .map(|title| outline::shown(&title, request.width)),
        });
    }
    let passages = reader.passages(&records);
    let chunk = pick.found.extent.records;
    let at = passages
        .iter()
        .filter(|passage| passage.range.first <= chunk.first)
        .rev()
        .find_map(|passage| passage.at)
        .unwrap_or_else(|| instant(pick.found.extent.date));
    let line = match list.line {
        Some(shows) => usize::try_from(chunk.first)
            .ok()
            .and_then(|first| records.get(first.checked_sub(1)?))
            .and_then(|record| shows(record)),
        None => outline::last_line(&passages, chunk.first, request.width),
    };
    Some(Said::Passage {
        records: chunk,
        day: day::utc(at),
        folder: reader.entry_labels(&records).into_iter().next(),
        line,
        summary: summary(request, entries, pick),
    })
}

fn summary(request: &Request<'_>, entries: &dyn Entries, pick: &Pick) -> Option<String> {
    let label = (request.way.label)(pick.found.entry);
    let head = entries.by_capture(&label).ok()??;
    let (newest, count) = (head.generation, head.records);
    if count == 0 {
        return None;
    }
    let notes = entries
        .read(head.id, newest, RecordRange { first: 1, count })
        .ok()?;
    let chunk = pick.found.extent.records;
    let (from, to) = (
        chunk.first,
        chunk.first.checked_add(chunk.count)?.checked_sub(1)?,
    );
    let mut best: Option<(usize, String)> = None;
    for record in &notes {
        let Some(written) = (request.way.written)(record) else {
            continue;
        };
        let mut shared: BTreeSet<u64> = BTreeSet::new();
        for source in &written.from {
            if source.entry != pick.found.entry || source.generation != pick.found.extent.generation
            {
                continue;
            }
            let Some(last) = source
                .records
                .first
                .checked_add(source.records.count)
                .and_then(|end| end.checked_sub(1))
            else {
                continue;
            };
            shared.extend(source.records.first.max(from)..=last.min(to));
        }
        if !shared.is_empty() && best.as_ref().is_none_or(|(most, _)| shared.len() >= *most) {
            best = Some((shared.len(), written.text));
        }
    }
    best.map(|(_, text)| text)
}
