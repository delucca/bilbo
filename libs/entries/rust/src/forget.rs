use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::fmt;

use bilbo_store::entries::{
    Capture, CaptureError, Captured, Entries, Generation, Head, Kept, Mark, Meta, StoreError,
};
use bilbo_store::id::Id;
use bilbo_store::labels::{CaptureLabel, Labels, StoredLabels};

use crate::read;
use crate::read::refs::Target;

pub const SCHEME: &str = "forgotten:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forgotten {
    pub id: Id,
    pub held: Mark,
    pub had: CaptureLabel,
}

impl Forgotten {
    #[must_use]
    pub fn label(&self) -> CaptureLabel {
        CaptureLabel::new(format!(
            "{SCHEME}{}:{}:{:032x}:{}",
            self.id,
            self.held.end,
            self.held.hash,
            self.had.as_str()
        ))
        .expect("a forgotten label is never empty")
    }

    #[must_use]
    pub fn read(id: Id, label: &CaptureLabel) -> Option<Forgotten> {
        let mut parts = label.as_str().strip_prefix(SCHEME)?.splitn(4, ':');
        let (_, end, digest, had) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let read = Forgotten {
            id,
            held: Mark {
                end: end.parse().ok()?,
                hash: u128::from_str_radix(digest, 16).ok()?,
            },
            had: CaptureLabel::new(had.to_string())?,
        };
        (read.label() == *label).then_some(read)
    }
}

#[must_use]
pub(crate) fn emptied(head: &Head) -> bool {
    head.generation.0 > 0 && head.records == 0
}

pub fn held(entries: &dyn Entries) -> Result<Vec<Forgotten>, StoreError> {
    Ok(entries
        .captures_with(SCHEME)?
        .into_iter()
        .filter_map(|(id, label)| Forgotten::read(id, &label))
        .collect())
}

pub type Labelled<'a> = dyn Fn(Id) -> Vec<CaptureLabel> + 'a;

#[must_use]
pub fn given_up(former: &[Forgotten], label: &CaptureLabel, held: Option<&Head>) -> bool {
    held.is_none() && former.iter().any(|forgotten| forgotten.had == *label)
}

pub fn given_up_by_lookup(
    entries: &dyn Entries,
    former: &[Forgotten],
    label: &CaptureLabel,
) -> Result<bool, StoreError> {
    if !former.iter().any(|forgotten| forgotten.had == *label) {
        return Ok(false);
    }
    Ok(given_up(former, label, entries.by_capture(label)?.as_ref()))
}

pub(crate) fn give_back(
    entries: &mut dyn Entries,
    former: &mut Vec<Forgotten>,
    id: Id,
    labelled: &Labelled<'_>,
) -> Result<u64, StoreError> {
    let mut bytes = 0;
    let mut asked = Vec::new();
    for label in labelled(id) {
        let held = entries.by_capture(&label)?;
        asked.push((label, held));
    }
    for (label, held) in asked {
        if !given_up(former, &label, held.as_ref()) {
            continue;
        }
        let Some(holder) = former
            .iter()
            .filter(|forgotten| forgotten.had == label)
            .map(|forgotten| forgotten.id)
            .max()
        else {
            continue;
        };
        let Some(meta) = entries.meta(holder)? else {
            continue;
        };
        bytes += entries.set_meta(
            holder,
            &Meta {
                labels: StoredLabels {
                    capture: Some(label),
                    ..meta.labels.clone()
                },
                ..meta
            },
        )?;
        former.retain(|forgotten| forgotten.id != holder);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub targets: Vec<Target>,
    pub listed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gone {
    pub id: Id,
    pub forgot: bool,
    pub held: Option<Generation>,
    pub kind: String,
    pub had: CaptureLabel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub gone: Gone,
    pub with: Vec<Gone>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub named: Vec<Named>,
}

#[derive(Debug)]
pub enum Error {
    Ref(read::Error),
    NoLabel(Id),
    NotEmptied {
        id: Id,
        named: Vec<Id>,
        cause: Option<Box<CaptureError>>,
    },
    Store {
        error: StoreError,
        named: Vec<Id>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ref(error) => write!(f, "{error}"),
            Self::NoLabel(id) => write!(
                f,
                "entry {id} has no capture label, which a forget is written under; nothing was forgotten"
            ),
            Self::NotEmptied { id, .. } => {
                write!(
                    f,
                    "entry {id} has its forgotten label, and the store did not empty it"
                )
            }
            Self::Store { error, .. } => write!(f, "{error}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Ref(error) => Some(error),
            Self::Store { error, .. } => Some(error),
            Self::NotEmptied { cause, .. } => cause
                .as_ref()
                .map(|cause| &**cause as &(dyn StdError + 'static)),
            Self::NoLabel(_) => None,
        }
    }
}

impl From<read::Error> for Error {
    fn from(error: read::Error) -> Self {
        match error {
            read::Error::Store(error) => Self::Store {
                error,
                named: Vec::new(),
            },
            other => Self::Ref(other),
        }
    }
}

pub fn resolve(request: &Request, entries: &dyn Entries) -> Result<Vec<(Id, Meta)>, read::Error> {
    read::resolve_each(&request.targets, request.listed, entries)
}

struct Plan {
    id: Id,
    meta: Meta,
    forgotten: Forgotten,
    relabel: bool,
    head: Option<Head>,
}

impl Plan {
    fn generation(&self) -> u32 {
        self.head.map_or(0, |head| head.generation.0)
    }

    fn records(&self) -> u64 {
        self.head.map_or(0, |head| head.records)
    }

    fn settled(&self) -> bool {
        !self.relabel && self.head.as_ref().is_some_and(emptied)
    }

    fn held(&self) -> Option<Generation> {
        match (self.generation(), self.records()) {
            (generation, records) if generation > 0 && records > 0 => Some(Generation(generation)),
            (generation, 0) if generation >= 2 => Some(Generation(generation - 1)),
            _ => None,
        }
    }

    fn done(&self, answer: &Captured) -> bool {
        let empty = |kept: &Kept, generation: u32| {
            kept.id == self.id
                && kept.generation == Generation(generation)
                && kept.records.count == 0
        };
        match (self.generation(), self.records(), answer) {
            (0, _, Captured::New(kept)) => empty(kept, 1),
            (generation, 0, Captured::Unchanged(id)) if generation > 0 => *id == self.id,
            (generation, records, Captured::NewGeneration(kept))
                if generation > 0 && records > 0 =>
            {
                generation
                    .checked_add(1)
                    .is_some_and(|next| empty(kept, next))
            }
            _ => false,
        }
    }
}

fn stopped(error: StoreError, named: &[Id]) -> Error {
    Error::Store {
        error,
        named: named.to_vec(),
    }
}

fn planned(
    store: &dyn Entries,
    id: Id,
    meta: Meta,
    label: CaptureLabel,
) -> Result<Plan, StoreError> {
    let head = store.by_capture(&label)?;
    Ok(match Forgotten::read(id, &label) {
        Some(forgotten) => Plan {
            id,
            meta,
            forgotten,
            relabel: false,
            head,
        },
        None => Plan {
            id,
            meta,
            forgotten: Forgotten {
                id,
                held: head.map_or_else(|| Mark::of(b""), |head| head.mark),
                had: label,
            },
            relabel: true,
            head,
        },
    })
}

fn belonging<S: Entries + Labels>(
    store: &S,
    id: Id,
    labelled: &Labelled<'_>,
    forgotten: &[Forgotten],
) -> Result<BTreeSet<Id>, StoreError> {
    let below = store.belonging_to(&[id])?;
    let mut found: BTreeSet<Id> = below.iter().copied().collect();
    for owner in std::iter::once(id).chain(below) {
        for label in labelled(owner) {
            if let Some(head) = store.by_capture(&label)? {
                found.insert(head.id);
            }
            found.extend(
                forgotten
                    .iter()
                    .filter(|forgotten| forgotten.had == label)
                    .map(|forgotten| forgotten.id),
            );
        }
    }
    found.remove(&id);
    Ok(found)
}

fn write<S: Entries>(store: &mut S, plan: &Plan, named: &[Id]) -> Result<Gone, Error> {
    let label = plan.forgotten.label();
    if plan.relabel {
        let mut meta = plan.meta.clone();
        meta.labels.capture = Some(label.clone());
        store
            .set_meta(plan.id, &meta)
            .map_err(|error| stopped(error, named))?;
    }
    if !plan.settled() {
        let answer = store
            .capture(&Capture {
                label: &label,
                kind: &plan.meta.kind,
                id: Some(plan.id),
                id_label: plan.meta.labels.id,
                tags: &plan.meta.labels.tags,
                stamp: None,
                bytes: &[],
                ends: &[],
            })
            .map_err(|error| match error {
                CaptureError::Store(error) => stopped(error, named),
                cause => Error::NotEmptied {
                    id: plan.id,
                    named: named.to_vec(),
                    cause: Some(Box::new(cause)),
                },
            })?;
        if !plan.done(&answer) {
            return Err(Error::NotEmptied {
                id: plan.id,
                named: named.to_vec(),
                cause: None,
            });
        }
    }
    Ok(Gone {
        id: plan.id,
        forgot: !plan.settled(),
        held: plan.held(),
        kind: plan.meta.kind.clone(),
        had: plan.forgotten.had.clone(),
    })
}

pub fn forget<S: Entries + Labels>(
    request: &Request,
    labelled: &Labelled<'_>,
    store: &mut S,
) -> Result<Output, Error> {
    let resolved = resolve(request, &*store)?;
    let mut owners = Vec::new();
    for (id, meta) in resolved {
        let label = meta.labels.capture.clone().ok_or(Error::NoLabel(id))?;
        owners.push((id, meta, label));
    }
    let named: Vec<Id> = owners.iter().map(|(id, _, _)| *id).collect();
    let forgotten = held(&*store).map_err(|error| stopped(error, &named))?;
    let mut taken: BTreeSet<Id> = named.iter().copied().collect();
    let mut groups = Vec::new();
    for (id, meta, label) in owners {
        let found =
            belonging(&*store, id, labelled, &forgotten).map_err(|error| stopped(error, &named))?;
        let owner = planned(&*store, id, meta, label).map_err(|error| stopped(error, &named))?;
        let mut members = Vec::new();
        for member in found {
            if !taken.insert(member) {
                continue;
            }
            let Some(meta) = store.meta(member).map_err(|error| stopped(error, &named))? else {
                continue;
            };
            let Some(label) = meta.labels.capture.clone() else {
                continue;
            };
            let plan =
                planned(&*store, member, meta, label).map_err(|error| stopped(error, &named))?;
            if !plan.settled() {
                members.push(plan);
            }
        }
        groups.push((owner, members));
    }
    let mut output = Output { named: Vec::new() };
    for (owner, members) in groups {
        let gone = write(store, &owner, &named)?;
        let mut with = Vec::new();
        for member in &members {
            with.push(write(store, member, &named)?);
        }
        output.named.push(Named { gone, with });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01JC5Q3V7XKM4P8R2T6W9YB0DE";
    const DIGEST: &str = "000a0b0c0d0e0f101112131415161718";

    fn id() -> Id {
        Id::parse(ID).expect("id")
    }

    fn label(text: &str) -> CaptureLabel {
        CaptureLabel::new(text.to_string()).expect("label")
    }

    #[test]
    fn a_forgotten_label_holds_the_id_the_mark_and_the_label_it_had() {
        let forgotten = Forgotten {
            id: id(),
            held: Mark {
                end: 412,
                hash: 0x000a_0b0c_0d0e_0f10_1112_1314_1516_1718,
            },
            had: label("file:/home/me/notes/shed.md"),
        };
        let made = forgotten.label();
        assert_eq!(
            made.as_str(),
            format!("forgotten:{ID}:412:{DIGEST}:file:/home/me/notes/shed.md")
        );
        assert_eq!(Forgotten::read(id(), &made), Some(forgotten));

        let odd = Forgotten {
            id: id(),
            held: Mark { end: 0, hash: 7 },
            had: label("file:/home/me/a:b\nc.md"),
        };
        assert_eq!(Forgotten::read(id(), &odd.label()), Some(odd));
    }

    #[test]
    fn a_text_that_is_not_the_form_is_no_forgotten_label() {
        let cases = [
            "file:/home/me/notes/shed.md".to_string(),
            "forgotten:".to_string(),
            format!("forgotten:{}:1:{DIGEST}:file:/a", &ID[..25]),
            format!("forgotten:{ID}:+1:{DIGEST}:file:/a"),
            format!("forgotten:{ID}:-1:{DIGEST}:file:/a"),
            format!("forgotten:{ID}:01:{DIGEST}:file:/a"),
            format!("forgotten:{ID}::{DIGEST}:file:/a"),
            format!("forgotten:{ID}:1:{}:file:/a", &DIGEST[1..]),
            format!("forgotten:{ID}:1:0{DIGEST}:file:/a"),
            format!("forgotten:{ID}:1:{}:file:/a", DIGEST.to_uppercase()),
            format!("forgotten:{ID}:1:{DIGEST}:"),
            format!("forgotten:{ID}:1:{DIGEST}"),
            format!("Forgotten:{ID}:1:{DIGEST}:file:/a"),
        ];
        for text in cases {
            assert_eq!(Forgotten::read(id(), &label(&text)), None, "{text}");
        }
    }

    #[test]
    fn a_forgotten_label_is_no_file_label() {
        let forgotten = Forgotten {
            id: id(),
            held: Mark { end: 3, hash: 9 },
            had: label("file:/home/me/notes/shed.md"),
        };
        assert_eq!(forgotten.label().path(), None);
    }
    #[test]
    fn a_forgotten_label_that_names_another_entry_is_none() {
        let other = Id::from_bytes([3; 16]);
        let made = Forgotten {
            id: id(),
            held: Mark { end: 3, hash: 9 },
            had: label("file:/home/me/notes/shed.md"),
        }
        .label();
        assert_eq!(Forgotten::read(other, &made), None);
        assert!(Forgotten::read(id(), &made).is_some());
    }
}
