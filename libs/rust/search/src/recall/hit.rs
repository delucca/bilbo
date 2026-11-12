use std::collections::HashMap;

use bilbo_kinds::reader::{Passage, Reader};
use bilbo_kinds::tokens::Tokens;
use bilbo_store::entries::{Entries, Generation, RecordRange};
use bilbo_store::id::Id;

use crate::chunk::{self, Size};
use crate::lookup::Found;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub found: Found,
    pub preface: Option<String>,
    pub text: String,
    pub cut: bool,
}

struct Held {
    generation: Generation,
    passages: Vec<Passage>,
    preface: Option<String>,
}

pub(super) struct Readback<'a> {
    entries: &'a dyn Entries,
    readers: &'a [Box<dyn Reader>],
    tokens: &'a dyn Tokens,
    size: Size,
    held: HashMap<Id, Option<Held>>,
}

fn load(entries: &dyn Entries, reader: &dyn Reader, id: Id) -> Option<Held> {
    let (generation, count) = entries.extent(id, None).ok()??;
    let records = entries
        .read(id, generation, RecordRange { first: 1, count })
        .ok()?;
    let records: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
    Some(Held {
        generation,
        passages: reader.passages(&records),
        preface: reader.preface(&records),
    })
}

impl<'a> Readback<'a> {
    pub(super) fn new(
        entries: &'a dyn Entries,
        readers: &'a [Box<dyn Reader>],
        tokens: &'a dyn Tokens,
        size: Size,
    ) -> Readback<'a> {
        Readback {
            entries,
            readers,
            tokens,
            size,
            held: HashMap::new(),
        }
    }

    pub(super) fn hit(&mut self, found: Found) -> Option<Hit> {
        let reader = self
            .readers
            .iter()
            .find(|reader| reader.kind() == found.kind)?;
        let entries = self.entries;
        let held = self
            .held
            .entry(found.entry)
            .or_insert_with(|| load(entries, reader.as_ref(), found.entry))
            .as_ref()?;
        if held.generation != found.extent.generation {
            return None;
        }
        let chunk = chunk::at(
            &held.passages,
            self.tokens,
            self.size,
            found.extent.records,
            found.extent.part,
        )
        .ok()??;
        if chunk::digest(&chunk.text) != found.digest {
            return None;
        }
        Some(Hit {
            preface: held.preface.clone(),
            text: chunk.text,
            cut: false,
            found,
        })
    }
}
