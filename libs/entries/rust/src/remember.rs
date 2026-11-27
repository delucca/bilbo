use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use bilbo_store::entries::{
    Capture, CaptureError, Captured, Entries, Generation, Head, Meta, Stamp, StoreError,
};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, StoredLabels, Tag, Tags};
use bilbo_store::opaque;

use crate::files::{At, Files, Node, at};
use crate::forget::{self, Forgotten};
use crate::kind::{Claim, Kind, Proposal};
use crate::tag::{self, Change};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    Pass,
    ByHand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub target: PathBuf,
    pub kind: Option<String>,
    pub tags: Tagging,
    pub run: Run,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tagging {
    pub tags: BTreeSet<Tag>,
    pub cap: usize,
}

pub struct Plan<'k> {
    /// Canonical, or a file that is gone in a canonical folder.
    pub target: PathBuf,
    /// The kind that runs; `None` when no kind claimed files.
    pub kind: Option<&'k dyn Kind>,
    /// The entries it claimed.
    pub claims: Vec<Claim>,
    /// What the walk could not read.
    pub refused: Vec<(PathBuf, String)>,
    pub tags: Tagging,
    pub run: Run,
    pub labelled: Option<&'k forget::Labelled<'k>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    New(Id),
    Appended(Id),
    NewGeneration(Id),
    Unchanged(Id),
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub id: Id,
    pub from: CaptureLabel,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Met {
    Own { id: Id, bytes: u64 },
    Again { id: Id, bytes: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub outcome: Outcome,
    pub moved: Option<Moved>,
    pub carried: Option<Id>,
    pub withdrew: Option<Generation>,
    pub forgotten: Option<Met>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub target: PathBuf,
    pub kind: Option<String>,
    /// 0 means nothing to keep.
    pub claimed: u64,
    pub items: Vec<Item>,
    pub records: u64,
    pub raw_bytes: u64,
    pub stored_bytes: u64,
    pub tagged: u64,
}

impl Output {
    #[must_use]
    pub fn unkept(plan: &Plan<'_>) -> Output {
        Output {
            target: plan.target.clone(),
            kind: plan.kind.map(|kind| kind.name().to_string()),
            claimed: 0,
            items: plan
                .refused
                .iter()
                .map(|(path, reason)| Item {
                    path: path.clone(),
                    outcome: Outcome::Refused(reason.clone()),
                    moved: None,
                    carried: None,
                    withdrew: None,
                    forgotten: None,
                })
                .collect(),
            records: 0,
            raw_bytes: 0,
            stored_bytes: 0,
            tagged: 0,
        }
    }

    #[must_use]
    pub fn tally(&self) -> Tally {
        let mut tally = Tally::default();
        for item in &self.items {
            match item.outcome {
                Outcome::New(_) => tally.new += 1,
                Outcome::Appended(_) => tally.appended += 1,
                Outcome::NewGeneration(_) => tally.new_generation += 1,
                Outcome::Unchanged(_) => tally.unchanged += 1,
                Outcome::Refused(_) => tally.refused += 1,
            }
        }
        tally
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub new: u64,
    pub appended: u64,
    pub new_generation: u64,
    pub unchanged: u64,
    pub refused: u64,
}

#[derive(Debug)]
pub enum Error {
    UnknownKind { name: String, known: Vec<String> },
    SeveralKinds { target: PathBuf, names: Vec<String> },
    Target { path: PathBuf, error: io::Error },
    Store(StoreError),
}

pub fn plan<'k>(
    request: &Request,
    kinds: &'k [Box<dyn Kind>],
    files: &dyn Files,
) -> Result<Plan<'k>, Error> {
    let named = match &request.kind {
        Some(name) => {
            let kind = kinds
                .iter()
                .find(|kind| kind.name() == name)
                .ok_or_else(|| Error::UnknownKind {
                    name: name.clone(),
                    known: kinds.iter().map(|kind| kind.name().to_string()).collect(),
                })?;
            Some(&**kind)
        }
        None => None,
    };
    match files.canonical(&request.target) {
        Ok(target) => claim_under(target, named, kinds, files, &request.tags, request.run),
        Err(error) => {
            if error.kind() == io::ErrorKind::NotFound
                && let Some(plan) = claim_gone(
                    &request.target,
                    named,
                    kinds,
                    files,
                    &request.tags,
                    request.run,
                )?
            {
                return Ok(plan);
            }
            Err(Error::Target {
                path: request.target.clone(),
                error,
            })
        }
    }
}

/// A file that is gone may still be an entry a kind claims through another of its files.
fn claim_gone<'k>(
    target: &Path,
    named: Option<&'k dyn Kind>,
    kinds: &'k [Box<dyn Kind>],
    files: &dyn Files,
    tags: &Tagging,
    run: Run,
) -> Result<Option<Plan<'k>>, Error> {
    let Some(name) = target.file_name() else {
        return Ok(None);
    };
    let dir = target
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Ok(folder) = files.canonical(dir) else {
        return Ok(None);
    };
    let plan = claim_under(folder.join(name), named, kinds, files, tags, run)?;
    Ok((!plan.claims.is_empty()).then_some(plan))
}

fn claim_under<'k>(
    target: PathBuf,
    named: Option<&'k dyn Kind>,
    kinds: &'k [Box<dyn Kind>],
    files: &dyn Files,
    tags: &Tagging,
    run: Run,
) -> Result<Plan<'k>, Error> {
    if let Some(kind) = named {
        let claims = kind.claims(files, &target);
        return Ok(Plan {
            target,
            kind: Some(kind),
            claims: claims.entries,
            refused: claims.refused,
            tags: tags.clone(),
            run,
            labelled: None,
        });
    }

    let mut claiming = Vec::new();
    let mut all_refused: Vec<(PathBuf, String)> = Vec::new();
    for kind in kinds {
        let claims = kind.claims(files, &target);
        for refusal in &claims.refused {
            if !all_refused.contains(refusal) {
                all_refused.push(refusal.clone());
            }
        }
        if !claims.entries.is_empty() {
            claiming.push((kind, claims));
        }
    }
    if claiming.len() > 1 {
        return Err(Error::SeveralKinds {
            target,
            names: claiming
                .iter()
                .map(|(kind, _)| kind.name().to_string())
                .collect(),
        });
    }
    let (kind, claimed, refused) = match claiming.pop() {
        Some((kind, claims)) => (Some(&**kind), claims.entries, claims.refused),
        None => (None, Vec::new(), all_refused),
    };
    Ok(Plan {
        target,
        kind,
        claims: claimed,
        refused,
        tags: tags.clone(),
        run,
        labelled: None,
    })
}

/// At most what keeping an entry holds for each record beside its bytes: the end `split` gives and what `capture`
/// keeps while it packs them. A kind that bounds what it unpacks counts it for every record.
pub const RECORD_HELD: u64 = 64;

/// Git's racy-index rule: a stamp this fresh may hide a same-size write in the same tick, so none
/// is kept.
const RACY: Duration = Duration::from_secs(2);

fn racy(modified: SystemTime, taken: SystemTime) -> bool {
    !taken.duration_since(modified).is_ok_and(|age| age >= RACY)
}

enum Done {
    Kept {
        captured: Captured,
        adopted: u64,
        tagged: bool,
    },
    Refused(String),
}

pub fn keep(
    plan: &Plan<'_>,
    files: &dyn Files,
    entries: &mut dyn Entries,
) -> Result<Output, Error> {
    let mut output = Output::unkept(plan);
    let Some(kind) = plan.kind else {
        return Ok(output);
    };
    output.claimed = u64::try_from(plan.claims.len()).unwrap_or(u64::MAX);
    let mut slot = None;
    for claim in &plan.claims {
        let named = plan.run == Run::ByHand
            && (plan.target == claim.label() || claim.files().contains(&plan.target));
        let (path, done, trail) = keep_file(kind, claim, plan, named, &mut slot, files, entries)?;
        output.stored_bytes += trail.moved.as_ref().map_or(0, |moved| moved.bytes);
        if let Some(Met::Again { bytes, .. }) = &trail.forgotten {
            output.stored_bytes += bytes;
        }
        let outcome = match done {
            Done::Refused(reason) => Outcome::Refused(reason),
            Done::Kept {
                captured,
                adopted,
                tagged,
            } => {
                output.stored_bytes += adopted;
                output.tagged += u64::from(tagged);
                let (outcome, kept) = match captured {
                    Captured::New(kept) => (Outcome::New(kept.id), Some(kept)),
                    Captured::Appended(kept) => (Outcome::Appended(kept.id), Some(kept)),
                    Captured::NewGeneration(kept) => (Outcome::NewGeneration(kept.id), Some(kept)),
                    Captured::Unchanged(id) => (Outcome::Unchanged(id), None),
                };
                if let Some(kept) = kept {
                    output.records += kept.records.count;
                    output.raw_bytes += kept.raw_bytes;
                    output.stored_bytes += kept.stored_bytes;
                }
                outcome
            }
        };
        output.items.push(Item {
            path,
            outcome,
            moved: trail.moved,
            carried: trail.carried,
            withdrew: None,
            forgotten: trail.forgotten,
        });
    }
    Ok(output)
}

#[derive(Default)]
struct Trail {
    moved: Option<Moved>,
    carried: Option<Id>,
    cap: Option<(PathBuf, String)>,
    forgotten: Option<Met>,
}

enum Step {
    Done(Done),
    Gone(io::Error),
    NotAFile,
}

/// Unchanged size and mtime: not read. The stamp is taken before the read, so a write during it
/// shows next run. An entry without an id label proposes one again, so a subagent kept before its
/// session gets it once it grows. A kind may move an entry from one of its files to another
/// between two looks, so a claim whose files are all gone is looked at once more.
fn keep_file(
    kind: &dyn Kind,
    claim: &Claim,
    plan: &Plan<'_>,
    named: bool,
    slot: &mut Option<Vec<Forgotten>>,
    files: &dyn Files,
    entries: &mut dyn Entries,
) -> Result<(PathBuf, Done, Trail), Error> {
    let label_path = claim.label().to_path_buf();
    let mut trail = Trail::default();
    let Some(label) = CaptureLabel::file(claim.label()) else {
        return Ok((
            label_path,
            Done::Refused("its path is not UTF-8".into()),
            trail,
        ));
    };
    let head = entries.by_capture(&label)?;
    if head.is_none() {
        listed(slot, entries)?;
    }
    let mut entry = Held {
        label: &label,
        head: head.as_ref(),
        tags: &plan.tags,
        forgotten: slot,
        named,
        labelled: plan.labelled,
    };
    let mut last = None;
    let mut not_a_file = false;
    for _ in 0..2 {
        for file in claim.files() {
            match look(kind, claim, file, &mut entry, files, entries, &mut trail)? {
                Step::Done(done) => {
                    if matches!(done, Done::Refused(_))
                        && trail.moved.is_none()
                        && let Some((path, reason)) = trail.cap.take()
                    {
                        return Ok((path, Done::Refused(reason), trail));
                    }
                    return Ok((file.clone(), done, trail));
                }
                Step::Gone(error) => last = Some(error),
                Step::NotAFile => not_a_file = true,
            }
        }
        if not_a_file {
            break;
        }
    }
    if let Some((path, reason)) = trail.cap.take() {
        return Ok((path, Done::Refused(reason), trail));
    }
    let reason = match last {
        Some(error) if !not_a_file => format!("cannot read: {error}"),
        _ => "it is not a regular file".to_owned(),
    };
    Ok((label_path, Done::Refused(reason), trail))
}

struct Held<'a> {
    label: &'a CaptureLabel,
    head: Option<&'a Head>,
    tags: &'a Tagging,
    forgotten: &'a mut Option<Vec<Forgotten>>,
    named: bool,
    labelled: Option<&'a forget::Labelled<'a>>,
}

fn look(
    kind: &dyn Kind,
    claim: &Claim,
    file: &Path,
    entry: &mut Held<'_>,
    files: &dyn Files,
    entries: &mut dyn Entries,
    trail: &mut Trail,
) -> Result<Step, Error> {
    match files.node(file) {
        Ok(Node::File) => {}
        Ok(_) => return Ok(Step::NotAFile),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Step::Gone(error)),
        Err(error) => return Ok(refused(format!("cannot read: {error}"))),
    }
    let stamp = match files.stamp(file) {
        Ok(stamp) => Some(stamp),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Step::Gone(error)),
        Err(_) => None,
    };
    let taken = files.now();
    let mut first = match fits(entry.tags, entry.head.map(|head| head.id), entries)? {
        Ok(first) => first,
        Err(reason) => {
            if entry.head.is_some() {
                return Ok(refused(reason));
            }
            if trail.cap.is_none() {
                trail.cap = Some((file.to_path_buf(), reason));
            }
            Tags::default()
        }
    };
    if entry.head.is_none()
        && !entry.named
        && let Some(id) = unread_forgotten(entry, stamp, entries)?
    {
        return Ok(Step::Done(Done::Kept {
            captured: Captured::Unchanged(id),
            adopted: 0,
            tagged: false,
        }));
    }
    if let Some(head) = entry.head
        && stamp.is_some()
        && head.stamp == stamp
    {
        return Ok(match declared(entry.tags, head.id, entries)? {
            Ok((tagged, bytes)) => Step::Done(Done::Kept {
                captured: Captured::Unchanged(head.id),
                adopted: bytes,
                tagged,
            }),
            Err(reason) => refused(reason),
        });
    }
    let bytes = match files.read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Step::Gone(error)),
        Err(error) => return Ok(refused(format!("cannot read: {error}"))),
    };
    let bytes = match kind.unpack(file, bytes) {
        Ok(bytes) => bytes,
        Err(refusal) => return Ok(refused(refusal.to_string())),
    };
    let ends = match kind.split(&bytes) {
        Ok(ends) => ends,
        Err(refusal) => return Ok(refused(refusal.to_string())),
    };
    let Some(records) = bytes.get(..ends.last().copied().unwrap_or(0)) else {
        return Ok(refused("its records do not cover it".into()));
    };
    let mut head = entry.head.copied();
    let proposal = if head.is_some() {
        Proposal {
            id: kind.propose(claim.label(), records, &|_| None).id,
            id_label: None,
        }
    } else {
        let entries: &dyn Entries = entries;
        kind.propose(claim.label(), records, &|other| kept(entries, other))
    };
    let holder = match proposal.id {
        Some(id) if head.is_none_or(|head| head.id != id) => entries.meta(id)?,
        _ => None,
    };
    if head.is_none() {
        let read = Read {
            records,
            ends: &ends,
            holder: holder.as_ref(),
            stamp: stamp.filter(|stamp| !racy(stamp.modified, taken)),
        };
        match passed_over(kind, file, entry, &read, files, entries, trail)? {
            Some(Meet::Over(step)) => return Ok(step),
            Some(Meet::Back(forgotten)) => match back(&forgotten, entry, entries, trail)? {
                Ok((kept, tags)) => {
                    head = Some(kept);
                    first = tags;
                }
                Err(reason) => return Ok(refused(reason)),
            },
            None => {}
        }
    }
    if let Some(id) = proposal.id
        && head.is_none_or(|head| head.id != id)
        && let Some(holder) = holder
    {
        let by = holder.labels.capture.clone();
        if let Some(head) = head {
            trail.carried = Some(id);
            return Ok(refused(format!(
                "{}, but this path was kept as entry {}",
                CaptureError::IdHeld { id, by },
                head.id
            )));
        }
        match moved(kind, holder, entry.label, file, files, entries, trail)? {
            Ok(kept) => head = Some(kept),
            Err(line) => return Ok(refused(line)),
        }
        first = match fits(entry.tags, head.map(|head| head.id), entries)? {
            Ok(first) => first,
            Err(reason) => return Ok(refused(reason)),
        };
    }
    if let Some((_, reason)) = &trail.cap {
        return Ok(refused(reason.clone()));
    }
    let input = Capture {
        label: entry.label,
        kind: kind.name(),
        id: if head.is_some() { None } else { proposal.id },
        id_label: if head.is_some() {
            None
        } else {
            proposal.id_label
        },
        stamp: stamp.filter(|stamp| !racy(stamp.modified, taken)),
        bytes: records,
        ends: &ends,
        tags: &first,
    };
    match entries.capture(&input) {
        Ok(captured) => {
            let (adopted, tagged) = match head {
                Some(head) => {
                    let adopted = adopt(kind, claim.label(), records, head.id, entries)?;
                    match declared(entry.tags, head.id, entries)? {
                        Ok((tagged, bytes)) => (adopted + bytes, tagged),
                        Err(reason) => return Ok(refused(reason)),
                    }
                }
                None => (
                    0,
                    matches!(captured, Captured::New(_)) && !entry.tags.tags.is_empty(),
                ),
            };
            Ok(Step::Done(Done::Kept {
                captured,
                adopted,
                tagged,
            }))
        }
        Err(CaptureError::Store(error)) => Err(error.into()),
        Err(
            error @ (CaptureError::IdHeld { .. }
            | CaptureError::Records(_)
            | CaptureError::TooLarge),
        ) => Ok(refused(error.to_string())),
    }
}

struct Read<'a> {
    records: &'a [u8],
    ends: &'a [usize],
    holder: Option<&'a Meta>,
    stamp: Option<Stamp>,
}

fn listed<'l>(
    slot: &'l mut Option<Vec<Forgotten>>,
    entries: &dyn Entries,
) -> Result<&'l mut Vec<Forgotten>, Error> {
    if slot.is_none() {
        *slot = Some(forget::held(entries)?);
    }
    Ok(slot.get_or_insert_default())
}

fn unread_forgotten(
    entry: &mut Held<'_>,
    stamp: Option<Stamp>,
    entries: &mut dyn Entries,
) -> Result<Option<Id>, Error> {
    let Some(stamp) = stamp else {
        return Ok(None);
    };
    let label = entry.label;
    for forgotten in listed(entry.forgotten, entries)?
        .iter()
        .filter(|forgotten| forgotten.had == *label)
    {
        if entries
            .by_capture(&forgotten.label())?
            .is_some_and(|head| forget::emptied(&head) && head.stamp == Some(stamp))
        {
            return Ok(Some(forgotten.id));
        }
    }
    Ok(None)
}

enum Meet {
    Over(Step),
    Back(Forgotten),
}

fn back(
    forgotten: &Forgotten,
    entry: &mut Held<'_>,
    entries: &mut dyn Entries,
    trail: &mut Trail,
) -> Result<Result<(Head, Tags), String>, Error> {
    let first = match fits(entry.tags, Some(forgotten.id), entries)? {
        Ok(first) => first,
        Err(reason) => {
            trail.cap = None;
            return Ok(Err(reason));
        }
    };
    let id = forgotten.id;
    let Some(meta) = entries.meta(id)? else {
        return Err(StoreError::Missing(format!("no entry {id}")).into());
    };
    let names = match entry.labelled {
        Some(labelled) => {
            let former = listed(entry.forgotten, &*entries)?;
            forget::give_back(entries, former, id, labelled)?
        }
        None => 0,
    };
    let bytes = names
        + entries.set_meta(
            id,
            &Meta {
                labels: StoredLabels {
                    capture: Some(entry.label.clone()),
                    ..meta.labels.clone()
                },
                ..meta
            },
        )?;
    trail.forgotten = Some(Met::Again { id, bytes });
    trail.cap = None;
    listed(entry.forgotten, entries)?.retain(|listed| listed.id != id);
    match entries.by_capture(entry.label)? {
        Some(head) => Ok(Ok((head, first))),
        None => Err(StoreError::Missing(format!("no entry {id} under {}", entry.label)).into()),
    }
}

fn passed_over(
    kind: &dyn Kind,
    file: &Path,
    entry: &mut Held<'_>,
    read: &Read<'_>,
    files: &dyn Files,
    entries: &mut dyn Entries,
    trail: &mut Trail,
) -> Result<Option<Meet>, Error> {
    let label = entry.label;
    let list = listed(entry.forgotten, entries)?;
    let mut same = None;
    if let Some(holder) = read.holder {
        let id = holder.id;
        if let Some(capture) = &holder.labels.capture
            && let Some(forgotten) = Forgotten::read(id, capture)
        {
            if holder.kind != kind.name() {
                trail.cap = None;
                return Ok(Some(Meet::Over(refused(
                    CaptureError::IdHeld {
                        id,
                        by: Some(capture.clone()),
                    }
                    .to_string(),
                ))));
            }
            same = Some(forgotten);
        }
    } else {
        for forgotten in list.iter().filter(|forgotten| forgotten.had == *label) {
            let Ok(end) = usize::try_from(forgotten.held.end) else {
                continue;
            };
            if end == 0
                || !read.ends.contains(&end)
                || !forgotten.held.begins(read.records)
                || entries
                    .meta(forgotten.id)?
                    .is_none_or(|meta| meta.kind != kind.name())
            {
                continue;
            }
            same = Some(forgotten.clone());
            break;
        }
    }
    let Some(mut forgotten) = same else {
        return Ok(None);
    };
    if entry.named {
        return Ok(Some(Meet::Back(forgotten)));
    }
    let mut bytes = 0;
    let displaced = read.holder.cloned();
    if forgotten.had != *label
        && let Some(meta) = displaced
    {
        let probe = Meta {
            labels: StoredLabels {
                capture: Some(forgotten.had.clone()),
                ..meta.labels.clone()
            },
            ..meta.clone()
        };
        if gives_way(kind, &probe, file, files) {
            forgotten.had = label.clone();
            bytes = entries.set_meta(
                forgotten.id,
                &Meta {
                    labels: StoredLabels {
                        capture: Some(forgotten.label()),
                        ..meta.labels
                    },
                    ..meta
                },
            )?;
            if let Some(listed) = list.iter_mut().find(|listed| listed.id == forgotten.id) {
                listed.had = label.clone();
            }
        }
    }
    if forgotten.had == *label
        && entries
            .by_capture(&forgotten.label())?
            .as_ref()
            .is_some_and(forget::emptied)
    {
        let own = forgotten.label();
        match entries.capture(&Capture {
            label: &own,
            kind: kind.name(),
            id: None,
            id_label: None,
            stamp: read.stamp,
            bytes: &[],
            ends: &[],
            tags: &Tags::default(),
        }) {
            Ok(_) => {}
            Err(CaptureError::Store(error)) => return Err(error.into()),
            Err(error) => {
                trail.cap = None;
                return Ok(Some(Meet::Over(refused(error.to_string()))));
            }
        }
    }
    trail.cap = None;
    trail.forgotten = Some(Met::Own {
        id: forgotten.id,
        bytes,
    });
    Ok(Some(Meet::Over(Step::Done(Done::Kept {
        captured: Captured::Unchanged(forgotten.id),
        adopted: bytes,
        tagged: false,
    }))))
}

fn moved(
    kind: &dyn Kind,
    holder: Meta,
    label: &CaptureLabel,
    file: &Path,
    files: &dyn Files,
    entries: &mut dyn Entries,
    trail: &mut Trail,
) -> Result<Result<Head, String>, Error> {
    let id = holder.id;
    let by = holder.labels.capture.clone();
    let from = by.clone().filter(|_| gives_way(kind, &holder, file, files));
    let Some(from) = from else {
        return Ok(Err(CaptureError::IdHeld { id, by }.to_string()));
    };
    let bytes = entries.set_meta(
        id,
        &Meta {
            labels: StoredLabels {
                capture: Some(label.clone()),
                ..holder.labels.clone()
            },
            ..holder
        },
    )?;
    trail.moved = Some(Moved { id, from, bytes });
    trail.cap = None;
    match entries.by_capture(label)? {
        Some(head) => Ok(Ok(head)),
        None => Err(StoreError::Missing(format!("no entry {id} under {label}")).into()),
    }
}

fn gives_way(kind: &dyn Kind, holder: &Meta, file: &Path, files: &dyn Files) -> bool {
    if holder.kind != kind.name() {
        return false;
    }
    let Some(old) = holder.labels.capture.as_ref().and_then(CaptureLabel::path) else {
        return false;
    };
    match at(files, old) {
        At::Nothing | At::NoFile => true,
        At::Elsewhere(canonical) if canonical == file => true,
        At::Unseen => false,
        At::File | At::Elsewhere(_) => {
            let Ok(bytes) = files.read(old) else {
                return false;
            };
            let Ok(bytes) = kind.unpack(old, bytes) else {
                return false;
            };
            let Ok(ends) = kind.split(&bytes) else {
                return false;
            };
            let Some(records) = bytes.get(..ends.last().copied().unwrap_or(0)) else {
                return false;
            };
            kind.propose(old, records, &|_| None).id != Some(holder.id)
        }
    }
}

fn refused(reason: String) -> Step {
    Step::Done(Done::Refused(reason))
}

fn full(tags: &Tagging) -> String {
    format!(
        "its entry would carry more than tag.cap tags ({})",
        tags.cap
    )
}

fn first_tags(tags: &Tagging) -> Result<Tags, String> {
    if tags.tags.is_empty() {
        return Ok(Tags::default());
    }
    let none = BTreeSet::new();
    let change = Change::Person {
        add: &tags.tags,
        remove: &none,
    };
    tag::apply(&Tags::default(), &change, tags.cap)
        .map(|applied| applied.tags)
        .map_err(|tag::Full| full(tags))
}

fn fits(
    tags: &Tagging,
    id: Option<Id>,
    entries: &dyn Entries,
) -> Result<Result<Tags, String>, Error> {
    let Some(id) = id else {
        return Ok(first_tags(tags));
    };
    if tags.tags.is_empty() {
        return Ok(Ok(Tags::default()));
    }
    let held = entries
        .meta(id)?
        .map(|meta| meta.labels.tags)
        .unwrap_or_default();
    let none = BTreeSet::new();
    let change = Change::Person {
        add: &tags.tags,
        remove: &none,
    };
    Ok(match tag::apply(&held, &change, tags.cap) {
        Ok(_) => Ok(Tags::default()),
        Err(tag::Full) => Err(full(tags)),
    })
}

fn declared(
    tags: &Tagging,
    id: Id,
    entries: &mut dyn Entries,
) -> Result<Result<(bool, u64), String>, Error> {
    if tags.tags.is_empty() {
        return Ok(Ok((false, 0)));
    }
    let none = BTreeSet::new();
    let change = Change::Person {
        add: &tags.tags,
        remove: &none,
    };
    match tag::declare(id, &change, tags.cap, entries) {
        Ok(declared) => Ok(Ok((declared.wrote, declared.bytes))),
        Err(tag::Error::Full(_)) => Ok(Err(full(tags))),
        Err(tag::Error::Gone(id)) => Err(StoreError::Missing(format!("no entry {id}")).into()),
        Err(tag::Error::Store(error)) => Err(error.into()),
    }
}

fn adopt(
    kind: &dyn Kind,
    path: &Path,
    records: &[u8],
    id: Id,
    entries: &mut dyn Entries,
) -> Result<u64, Error> {
    let Some(meta) = entries.meta(id)?.filter(|meta| meta.labels.id.is_none()) else {
        return Ok(0);
    };
    let found = {
        let entries: &dyn Entries = entries;
        kind.propose(path, records, &|other| kept(entries, other))
            .id_label
            .filter(|found| *found != id)
    };
    let Some(found) = found else {
        return Ok(0);
    };
    Ok(entries.set_meta(
        id,
        &Meta {
            labels: StoredLabels {
                id: Some(found),
                ..meta.labels
            },
            ..meta
        },
    )?)
}

pub struct Written<'a> {
    pub kind: &'a str,
    pub label: &'a CaptureLabel,
    pub id: Id,
    pub record: &'a [u8],
    pub tags: &'a Tagging,
}

#[derive(Debug)]
pub enum WrittenAnswer {
    Kept {
        id: Id,
        records: u64,
        raw_bytes: u64,
        stored_bytes: u64,
        tagged: bool,
    },
    Refused(String),
    BeforeCapture(StoreError),
    AtCapture {
        id: Id,
        error: Attempt,
    },
}

#[derive(Debug)]
pub enum Attempt {
    Store(StoreError),
    NoNewEntry,
}

impl StdError for Attempt {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::NoNewEntry => None,
        }
    }
}

impl fmt::Display for Attempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::NoNewEntry => f.write_str("the store did not make a new entry for the note"),
        }
    }
}

pub fn keep_written(entries: &mut dyn Entries, written: &Written<'_>) -> WrittenAnswer {
    let first = match first_tags(written.tags) {
        Ok(first) => first,
        Err(reason) => return WrittenAnswer::Refused(reason),
    };
    if opaque::filter(written.record).as_ref() != written.record {
        return WrittenAnswer::Refused(
            "the store would keep only a placeholder for its text, which it takes for encoded data"
                .to_owned(),
        );
    }
    match entries.by_capture(written.label) {
        Ok(None) => {}
        Ok(Some(_)) => {
            return WrittenAnswer::Refused(format!(
                "its id {} is taken by another entry",
                written.id
            ));
        }
        Err(error) => return WrittenAnswer::BeforeCapture(error),
    }
    let input = Capture {
        label: written.label,
        kind: written.kind,
        id: Some(written.id),
        id_label: None,
        stamp: None,
        bytes: written.record,
        ends: &[written.record.len()],
        tags: &first,
    };
    match entries.capture(&input) {
        Ok(Captured::New(kept)) => WrittenAnswer::Kept {
            id: kept.id,
            records: kept.records.count,
            raw_bytes: kept.raw_bytes,
            stored_bytes: kept.stored_bytes,
            tagged: !written.tags.tags.is_empty(),
        },
        Ok(_) => WrittenAnswer::AtCapture {
            id: written.id,
            error: Attempt::NoNewEntry,
        },
        Err(CaptureError::Store(error)) => WrittenAnswer::AtCapture {
            id: written.id,
            error: Attempt::Store(error),
        },
        Err(error) => WrittenAnswer::Refused(error.to_string()),
    }
}

/// A failed lookup gives no id: the capture that follows reads the same catalog and reports it.
fn kept(entries: &dyn Entries, path: &Path) -> Option<Id> {
    let label = CaptureLabel::file(path)?;
    entries
        .by_capture(&label)
        .ok()
        .flatten()
        .map(|head| head.id)
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind { name, known } => {
                write!(f, "unknown kind '{name}'; kinds: {}", known.join(", "))
            }
            Self::SeveralKinds { target, names } => write!(
                f,
                "several kinds claim files under {}: {}; choose one with --kind",
                target.display(),
                list(names)
            ),
            Self::Target { path, error } => {
                write!(f, "cannot read {}: {error}", path.display())
            }
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Target { error, .. } => Some(error),
            Self::Store(error) => Some(error),
            Self::UnknownKind { .. } | Self::SeveralKinds { .. } => None,
        }
    }
}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};

    use bilbo_store::entries::StoreError;

    use super::{Error, Output, Plan, Request, Tagging, plan};
    use crate::fake::Suffix;
    use crate::files::{Files, Memory};
    use crate::kind::{Claim, Claims, Kind, Proposal, Refusal};

    fn registry(kinds: Vec<Suffix>) -> Vec<Box<dyn Kind>> {
        kinds
            .into_iter()
            .map(|kind| Box::new(kind) as Box<dyn Kind>)
            .collect()
    }

    fn request(target: &str, kind: Option<&str>) -> Request {
        Request {
            target: PathBuf::from(target),
            kind: kind.map(str::to_string),
            tags: Tagging::default(),
            run: crate::remember::Run::ByHand,
        }
    }

    fn tree() -> Memory {
        Memory::new()
            .file("/t/a.one", "a")
            .file("/t/sub/b.one", "b")
            .file("/t/c.two", "c")
            .file("/t/d.txt", "d")
    }

    struct Resolving {
        files: Memory,
        from: PathBuf,
        to: PathBuf,
    }

    impl Resolving {
        fn new(files: Memory, from: &str, to: &str) -> Resolving {
            Resolving {
                files,
                from: PathBuf::from(from),
                to: PathBuf::from(to),
            }
        }
    }

    impl Files for Resolving {
        fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
            if path == self.from {
                Ok(self.to.clone())
            } else {
                self.files.canonical(path)
            }
        }
        fn node(&self, path: &Path) -> io::Result<crate::files::Node> {
            self.files.node(path)
        }
        fn list(&self, folder: &Path) -> io::Result<Vec<std::ffi::OsString>> {
            self.files.list(folder)
        }
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.files.read(path)
        }
        fn stamp(&self, path: &Path) -> io::Result<bilbo_store::entries::Stamp> {
            self.files.stamp(path)
        }
    }

    fn labels(plan: &Plan<'_>) -> Vec<PathBuf> {
        plan.claims
            .iter()
            .map(|claim| claim.label().to_path_buf())
            .collect()
    }

    fn running<'a>(plan: &'a Plan<'_>) -> Option<&'a str> {
        plan.kind.map(Kind::name)
    }

    #[test]
    fn detection_runs_the_one_kind_that_claims_files() {
        let kinds = registry(vec![Suffix::new("one", ".one"), Suffix::new("two", ".two")]);
        let plan = plan(&request("/t/sub", None), &kinds, &tree()).expect("plan");
        assert_eq!(running(&plan), Some("one"));
        assert_eq!(labels(&plan), vec![PathBuf::from("/t/sub/b.one")]);
        assert_eq!(plan.target, PathBuf::from("/t/sub"));
    }

    #[test]
    fn detection_with_no_claiming_kind_has_nothing_to_keep() {
        let mut one = Suffix::new("one", ".one");
        one.refused
            .push((PathBuf::from("/t/x"), "cannot read".into()));
        let mut two = Suffix::new("two", ".two");
        two.refused
            .push((PathBuf::from("/t/x"), "cannot read".into()));
        two.refused
            .push((PathBuf::from("/t/y"), "cannot read".into()));
        let kinds = registry(vec![one, two]);
        let plan = plan(&request("/t/d.txt", None), &kinds, &tree()).expect("plan");
        assert!(plan.kind.is_none());
        assert!(plan.claims.is_empty());
        assert_eq!(
            plan.refused,
            vec![
                (PathBuf::from("/t/x"), "cannot read".to_string()),
                (PathBuf::from("/t/y"), "cannot read".to_string()),
            ]
        );
        let output = Output::unkept(&plan);
        assert_eq!(output.claimed, 0);
        assert_eq!(output.kind, None);
    }

    #[test]
    fn detection_with_several_claiming_kinds_names_them() {
        let kinds = registry(vec![
            Suffix::new("one", ".one"),
            Suffix::new("none", ".none"),
            Suffix::new("two", ".two"),
        ]);
        let error = plan(&request("/t", None), &kinds, &tree())
            .err()
            .expect("several kinds claim files");
        assert_eq!(
            error.to_string(),
            "several kinds claim files under /t: one and two; choose one with --kind"
        );
    }

    #[test]
    fn only_the_running_kinds_refusals_are_kept_when_one_kind_claims() {
        let mut quiet = Suffix::new("two", ".two");
        quiet.refused = vec![(PathBuf::from("/t/locked"), "denied".to_owned())];
        let mut running_kind = Suffix::new("one", ".one");
        running_kind.refused = vec![(PathBuf::from("/t/other"), "unreadable".to_owned())];
        let kinds = registry(vec![running_kind, quiet]);
        let files = Memory::new().file("/t/a.one", "x");
        let plan = plan(&request("/t", None), &kinds, &files).expect("plan");
        assert_eq!(running(&plan), Some("one"));
        assert_eq!(
            plan.refused,
            vec![(PathBuf::from("/t/other"), "unreadable".to_owned())]
        );

        let none = Memory::new().file("/t/a.txt", "x");
        let plan = self::plan(&request("/t", None), &kinds, &none).expect("plan");
        assert_eq!(plan.refused.len(), 2);
    }

    #[test]
    fn a_named_kind_runs_alone_even_when_others_claim() {
        let kinds = registry(vec![Suffix::new("one", ".one"), Suffix::new("two", ".two")]);
        let plan = plan(&request("/t", Some("two")), &kinds, &tree()).expect("plan");
        assert_eq!(running(&plan), Some("two"));
        assert_eq!(labels(&plan), vec![PathBuf::from("/t/c.two")]);

        let empty = self::plan(&request("/t/d.txt", Some("two")), &kinds, &tree()).expect("plan");
        assert_eq!(running(&empty), Some("two"));
        assert!(empty.claims.is_empty());
    }

    #[test]
    fn an_unknown_kind_names_the_known_ones() {
        let kinds = registry(vec![Suffix::new("one", ".one"), Suffix::new("two", ".two")]);
        let error = plan(&request("/nowhere", Some("pdf")), &kinds, &tree())
            .err()
            .expect("unknown kind");
        assert_eq!(error.to_string(), "unknown kind 'pdf'; kinds: one, two");
    }

    #[test]
    fn a_target_that_cannot_be_read_is_an_error() {
        let kinds = registry(vec![Suffix::new("one", ".one")]);
        let error = plan(&request("/nowhere", None), &kinds, &tree())
            .err()
            .expect("missing target");
        assert!(matches!(&error, Error::Target { path, .. } if path == Path::new("/nowhere")));
    }

    #[test]
    fn kinds_see_the_canonical_target() {
        let kinds = registry(vec![Suffix::new("one", ".one")]);
        let plan = plan(
            &request("/link", None),
            &kinds,
            &Resolving::new(tree(), "/link", "/t/sub"),
        )
        .expect("plan");
        assert_eq!(plan.target, PathBuf::from("/t/sub"));
        assert_eq!(labels(&plan), vec![PathBuf::from("/t/sub/b.one")]);
    }

    #[test]
    fn a_bare_name_that_is_gone_is_looked_for_in_the_current_folder() {
        struct Anything;
        impl Kind for Anything {
            fn name(&self) -> &'static str {
                "anything"
            }
            fn claims(&self, _: &dyn Files, target: &Path) -> Claims {
                Claims {
                    entries: vec![Claim::file(target.to_path_buf())],
                    refused: Vec::new(),
                }
            }
            fn split(&self, _: &[u8]) -> Result<Vec<usize>, Refusal> {
                Ok(Vec::new())
            }
            fn propose(
                &self,
                _: &Path,
                _: &[u8],
                _: &dyn Fn(&Path) -> Option<bilbo_store::id::Id>,
            ) -> Proposal {
                Proposal::default()
            }
        }
        let kinds: Vec<Box<dyn Kind>> = vec![Box::new(Anything)];
        let plan = plan(
            &request("gone.one", None),
            &kinds,
            &Resolving::new(tree(), ".", "/t"),
        )
        .expect("plan");
        assert_eq!(plan.target, PathBuf::from("/t/gone.one"));
        assert_eq!(labels(&plan), vec![PathBuf::from("/t/gone.one")]);
    }

    #[test]
    fn the_use_case_errors_say_what_went_wrong() {
        let names = |names: &[&str]| names.iter().map(|n| (*n).to_string()).collect::<Vec<_>>();
        let several = |list: &[&str]| {
            Error::SeveralKinds {
                target: PathBuf::from("/t"),
                names: names(list),
            }
            .to_string()
        };
        assert_eq!(
            Error::UnknownKind {
                name: "pdf".into(),
                known: names(&["markdown", "jsonl"]),
            }
            .to_string(),
            "unknown kind 'pdf'; kinds: markdown, jsonl"
        );
        assert_eq!(
            several(&["a", "b"]),
            "several kinds claim files under /t: a and b; choose one with --kind"
        );
        assert_eq!(
            several(&["a", "b", "c"]),
            "several kinds claim files under /t: a, b and c; choose one with --kind"
        );
        assert_eq!(
            Error::Target {
                path: PathBuf::from("/t/x"),
                error: io::Error::from(io::ErrorKind::NotFound),
            }
            .to_string(),
            format!(
                "cannot read /t/x: {}",
                io::Error::from(io::ErrorKind::NotFound)
            )
        );
        assert_eq!(
            Error::from(StoreError::ReadOnly).to_string(),
            "the store is open read-only"
        );
    }
}
